//! Compatibility with the Python tools.  The reference values come from
//! tools/pc1.py and tools/chart.py; the full test is `teasi check <chart>`
//! over the original files, which needs chart data and therefore lives in the
//! CLI rather than here.

use md5::{Digest, Md5};
use teasi::{chart, layers, lzma, osmpoi, pc1, ta_index};

fn key() -> Vec<u8> {
    (0..32u8).collect()
}

#[test]
fn pc1_matches_python() {
    let data: Vec<u8> = (0..16u8).collect();
    assert_eq!(
        hex(&pc1::encrypt_blob(&data, &key())),
        "14b2a04a6b17e02bc9c31de41478f6d1"
    );
    // partial: bytes 0..99, then every 10th
    let data: Vec<u8> = (0..120u8).collect();
    assert_eq!(
        format!("{:x}", Md5::digest(pc1::encrypt_payload(&data, &key()))),
        "256b50b38d66e4cee76cfbf96bcaad86"
    );
}

#[test]
fn pc1_round_trip() {
    let data: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
    assert_eq!(pc1::decrypt_blob(&pc1::encrypt_blob(&data, &key()), &key()), data);
    assert_eq!(pc1::decrypt_payload(&pc1::encrypt_payload(&data, &key()), &key()), data);
}

#[test]
fn lzma_matches_python() {
    let raw: Vec<u8> = (0..=255u8).cycle().take(256 * 40).collect();
    let c = lzma::compress(&raw).unwrap();
    assert_eq!(c.len(), 282);
    assert_eq!(format!("{:x}", Md5::digest(&c)), "b80393d3f10f0677ae40fd13b9b120af");
    assert_eq!(lzma::decompress(&c, raw.len()).unwrap(), raw);
}

#[test]
fn global_key_follows_the_firmware() {
    assert_eq!(chart::global_key(b"2013021200000368"), chart::STATIC_KEY);
    assert_eq!(
        String::from_utf8(chart::global_key(b"9999999900000001")).unwrap(),
        "EF775988943825D2871E1CFA75473EC0"
    );
}

#[test]
fn d_record_round_trip() {
    // one a5 item (an osmpoi POI): name and attribute strings as variable parts
    let name: Vec<u16> = "Café\0".encode_utf16().collect();
    let attrs: Vec<u16> = "00Langgade 35\0".encode_utf16().collect();
    let item = layers::Item {
        s: vec![0, name.len() as u32, 0, attrs.len() as u32, 14, 0x1234_5678],
        v: vec![layers::Var::U16(name), layers::Var::U16(attrs)],
    };
    let mut hdr = vec![0u32; layers::D_HDR];
    hdr[0] = 0x18;
    hdr[11] = 1;
    let rec = layers::ArrayRec { hdr, arrays: vec![vec![], vec![], vec![], vec![], vec![item]] };
    let raw = layers::build_arrays(&rec, &layers::D_SPEC);
    assert_eq!(layers::parse_d(&raw).unwrap(), rec);
}

#[test]
fn a_record_round_trip() {
    let item = layers::AItem {
        s: vec![0, 3, 1, 1, 0, 1, 0],
        name: "DK\0".encode_utf16().collect(),
        s18: vec![layers::Sub18 { s: vec![0, 4, 0, 2, 7, 0], text: "Fyn\0".encode_utf16().collect() }],
        s10: vec![layers::Sub10 {
            s: vec![0, 3, 2, 0],
            text: "AB\0".encode_utf16().collect(),
            nums: vec![11, 22],
        }],
    };
    let body = layers::build_a(&layers::ARec {
        hdr: vec![0, 0, 0, 1, 0, 0, 0],
        items: vec![item.clone()],
        blk5: vec![],
        blk6: vec![],
    });
    // blk5 / blk6 start where the items end, so the offsets follow from that length
    let end = body.len() as u32;
    let rec = layers::ARec {
        hdr: vec![0, 0, 0, 1, 0, end, end + 4],
        items: vec![item],
        blk5: vec![1, 2, 3, 4],
        blk6: vec![5, 6, 7, 8],
    };
    assert_eq!(layers::parse_a(&layers::build_a(&rec)).unwrap(), rec);
}

#[test]
fn index_round_trip() {
    let res = ta_index::Res {
        typ: 0,
        name: ta_index::Name::Inline(b"Odense".to_vec()),
        lon: 10.4,
        lat: 55.4,
        cells: vec![ta_index::Cell { x: 501, y: 81, offs: vec![] }],
    };
    let leaf = ta_index::Node { kids: vec![], res: vec![res.clone()] };
    let root = ta_index::Node {
        kids: vec![ta_index::Kid { ch: 'o' as u16, mask: ta_index::ALL, node: leaf }],
        res: vec![],
    };
    let idx = ta_index::Index {
        langs: vec![(*b"DAN", 1), (*b"GER", 2)],
        one: 1,
        country: "Denmark".encode_utf16().collect(),
        root,
        pool: vec![],
    };
    let raw = ta_index::build(&idx).unwrap();
    let back = ta_index::parse(&raw).unwrap();
    assert_eq!(back, idx);
    assert_eq!(back.count(), 1);
    assert_eq!(ta_index::build(&back).unwrap(), raw);
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

#[test]
fn teasi_units_match_python() {
    // reference values from tools/osm_addr_extract.py (SCALE = 2**28 / 360)
    for (dm_lon, dm_lat, bx, by) in [
        (102345678i32, 553456789i32, 0x41a0e8e4adbf157fu64, 0x4178a4a06af89798u64),
        (-17654321, 603456789, 0x419fafa739a863bc, 0x41751667876a5eb5),
        (0, 0, 0x41a0000000000000, 0x4190000000000000),
    ] {
        let (x, y) = teasi::pbf::xy_dm(dm_lon, dm_lat);
        assert_eq!(x.to_bits(), bx, "x for {} {}", dm_lon, dm_lat);
        assert_eq!(y.to_bits(), by, "y for {} {}", dm_lon, dm_lat);
    }
    assert_eq!(teasi::pbf::SCALE.to_bits(), 0x4126c16c16c16c17);
}

#[test]
fn node_index_keeps_what_it_was_asked_for() {
    let mut ix = teasi::pbf::NodeIndex::new(vec![7, 3, 7, 1]);
    assert_eq!(ix.len(), 3);
    assert!(ix.wants(3) && !ix.wants(4));
    assert_eq!(ix.missing(), 3);
    ix.set(3, 102345678, 553456789);
    ix.set(4, 0, 0); // not asked for: ignored
    assert_eq!(ix.missing(), 2);
    assert_eq!(ix.get(3), Some(teasi::pbf::xy_dm(102345678, 553456789)));
    assert_eq!(ix.get(7), None);
}

#[test]
fn fsum_matches_pythons_sum() {
    // CPython's sum() compensates since 3.12; a naive loop gives 0.0 and
    // 0.9999999999999999 here
    assert_eq!(teasi::pbf::fsum([1.0, 1e16, 1.0, -1e16].into_iter()), 2.0);
    assert_eq!(teasi::pbf::fsum(std::iter::repeat_n(0.1, 10)).to_bits(), 1.0f64.to_bits());
}

#[test]
fn segments_build_rings_and_cancel_in_pairs() {
    use teasi::pbf::assemble_segments;
    let p = |x: i32, y: i32| (x, y);
    // two open ways forming one ring, the second one reversed
    let ways = vec![
        vec![p(0, 0), p(4, 0), p(4, 4)],
        vec![p(0, 4), p(4, 4)],
        vec![p(0, 0), p(0, 4)],
    ];
    let rings = assemble_segments(&ways).unwrap();
    assert_eq!(rings.len(), 1);
    assert_eq!(rings[0].len(), 4);
    // a closed way is a ring of its own
    assert_eq!(
        assemble_segments(&[vec![p(0, 0), p(2, 0), p(2, 2), p(0, 0)]]),
        Some(vec![vec![p(0, 0), p(2, 0), p(2, 2)]])
    );
    // two squares sharing an edge: the shared segments cancel, one ring is left
    let a = vec![p(0, 0), p(4, 0), p(4, 4), p(0, 4), p(0, 0)];
    let b = vec![p(4, 0), p(8, 0), p(8, 4), p(4, 4), p(4, 0)];
    let rings = assemble_segments(&[a, b]).unwrap();
    assert_eq!(rings.len(), 1);
    assert_eq!(rings[0].len(), 6, "the shared edge is gone: {:?}", rings[0]);
    // a spike that doubles back disappears with its mirror segment
    let spike = vec![p(0, 0), p(4, 0), p(6, 0), p(4, 0), p(4, 4), p(0, 0)];
    assert_eq!(assemble_segments(&[spike]).unwrap()[0].len(), 3);
    // an open ring builds no area at all
    assert_eq!(assemble_segments(&[vec![p(0, 0), p(1, 0), p(1, 1)]]), None);
}

#[test]
fn a_ring_touching_itself_is_split() {
    use teasi::pbf::split_rings;
    let p = |x: i32, y: i32| (x, y);
    // a figure of eight through (4,4): two rings, each starting at the touch
    let eight =
        vec![p(0, 0), p(4, 0), p(4, 4), p(8, 4), p(8, 8), p(4, 8), p(4, 4), p(0, 4)];
    assert_eq!(
        split_rings(&eight),
        vec![
            vec![p(4, 4), p(8, 4), p(8, 8), p(4, 8)],
            vec![p(0, 0), p(4, 0), p(4, 4), p(0, 4)],
        ]
    );
    // a plain ring stays whole, a two point stub is dropped
    assert_eq!(split_rings(&[p(0, 0), p(1, 0), p(1, 1)]).len(), 1);
    assert!(split_rings(&[p(0, 0), p(1, 0)]).is_empty());
}

#[test]
fn an_inner_ring_does_not_count_towards_the_centre() {
    use teasi::pbf::area_centre;
    // a 10 x 10 square with a small ring inside it: the centre is the square's
    let outer = vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (0.0, 0.0)];
    let inner = vec![(4.0, 4.0), (6.0, 4.0), (6.0, 6.0), (4.0, 6.0), (4.0, 4.0)];
    let only_outer = area_centre(std::slice::from_ref(&outer)).unwrap();
    assert_eq!(area_centre(&[outer.clone(), inner]), Some(only_outer));
    // the first point counts twice, which is what libosmium's rings do
    assert_eq!(only_outer, (4.0, 4.0));
    assert_eq!(area_centre(&[]), None);
}

#[test]
fn probe_finds_the_same_ids_as_a_binary_search() {
    let ids: Vec<i64> = (0..500).map(|i| i * 7 + 3).collect();
    let ix = teasi::pbf::NodeIndex::new(ids.clone());
    let mut p = ix.probe();
    for id in 0..3600i64 {
        assert_eq!(p.wants(id), ix.wants(id), "ascending {}", id);
    }
    let mut p = ix.probe();
    for id in (0..3600i64).rev() {
        assert_eq!(p.wants(id), ix.wants(id), "descending {}", id);
    }
    let mut p = ix.probe();
    for id in [3500i64, 3, 1750, 10, 3503, -5, 3496] {
        assert_eq!(p.wants(id), ix.wants(id), "jumping {}", id);
    }
    let empty = teasi::pbf::NodeIndex::new(vec![]);
    assert!(!empty.probe().wants(5));
}

// --------------------------------------------------------------------------
// osmpoi (tools/compile_osmpoi.py)
// --------------------------------------------------------------------------

fn tags<'a>(pairs: &'a [(&'a str, &'a str)]) -> osmpoi::Tags<'a> {
    osmpoi::tags_of(pairs.iter().copied())
}

#[test]
fn poi_rules_match_python() {
    // first match in RULES wins, and the two early returns do not fall through
    assert_eq!(osmpoi::poi_type(&tags(&[("amenity", "fuel")]), true), Some(0x0B));
    assert_eq!(
        osmpoi::poi_type(&tags(&[("amenity", "cafe"), ("shop", "bicycle")]), true),
        Some(0x01) // shop=bicycle is listed before amenity=cafe
    );
    assert_eq!(
        osmpoi::poi_type(&tags(&[("amenity", "place_of_worship"), ("religion", "jewish")]), true),
        Some(0x24)
    );
    assert_eq!(
        osmpoi::poi_type(&tags(&[("amenity", "place_of_worship"), ("shop", "bicycle")]), true),
        None // place_of_worship without a known religion is not a POI
    );
    // aerodromes only count as nodes
    assert_eq!(osmpoi::poi_type(&tags(&[("aeroway", "aerodrome")]), true), Some(0x27));
    assert_eq!(osmpoi::poi_type(&tags(&[("aeroway", "aerodrome")]), false), None);
    // historic=* unless it is one of the skipped values
    assert_eq!(osmpoi::poi_type(&tags(&[("historic", "castle")]), true), Some(0x48));
    assert_eq!(osmpoi::poi_type(&tags(&[("historic", "wall")]), true), None);
    assert_eq!(osmpoi::poi_type(&tags(&[("name", "nowhere")]), true), None);
}

#[test]
fn poi_attributes_match_python() {
    let t = tags(&[
        ("addr:street", "Hauptstr."),
        ("addr:housenumber", "7"),
        ("addr:city", "Esbjerg"),
        ("opening_hours", "Mo-Fr 08:00-18:00; Sa 09:00-13:00"),
        ("contact:phone", "+45 1"),
        ("access", "yes"),
    ]);
    assert_eq!(
        osmpoi::attributes(&t),
        "00Hauptstr. 7\nEsbjerg|02+45 1|03Mo-Fr 08:00-18:00\nSa 09:00-13:00|74yes"
    );
    // no address tags at all: no 00 entry
    assert_eq!(osmpoi::attributes(&tags(&[("website", "x")])), "01x");
    assert_eq!(osmpoi::attributes(&tags(&[])), "");
}

#[test]
fn osmpoi_record_round_trip() {
    let pois = vec![
        layers::PoiItem { typ: 0x0B, pos: 0x1234_5678, name: "Tankstelle".into(), attrs: "74yes".into() },
        layers::PoiItem { typ: 0x1D, pos: 1, name: "Kirche \u{1F600}".into(), attrs: String::new() },
    ];
    let raw = layers::build_osmpoi(&pois);
    assert_eq!(layers::parse_osmpoi(&raw).unwrap(), pois);
    // the header announces the array size and the item count, nothing else
    let rec = layers::parse_d(&raw).unwrap();
    assert_eq!(rec.hdr[0], 2 * 0x18);
    assert_eq!(rec.hdr[11], 2);
}

#[test]
fn poi_centroid_matches_python() {
    // unit square: centroid in the middle, whichever vertex it starts at
    let sq = [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)];
    assert_eq!(teasi::poi::centroid(&sq), Some((1.0, 1.0)));
    assert_eq!(teasi::poi::centroid(&[(0.0, 0.0), (1.0, 1.0)]), None); // degenerate
}

#[test]
fn poly_contains_counts_holes() {
    use teasi::poly::Ring;
    let outer = Ring { pts: vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)], hole: false };
    let hole = Ring { pts: vec![(4.0, 4.0), (6.0, 4.0), (6.0, 6.0), (4.0, 6.0)], hole: true };
    let rings = vec![outer, hole];
    assert!(teasi::poly::contains(&rings, 1.0, 1.0));
    assert!(!teasi::poly::contains(&rings, 5.0, 5.0));
    assert!(!teasi::poly::contains(&rings, 11.0, 5.0));
}

// --------------------------------------------------------------------------
// osmpoint (tools/compile_osmpoint.py); all expected values printed by it
// --------------------------------------------------------------------------

fn seamark(pairs: &[(&str, &str)]) -> teasi::pbf::TagMap {
    teasi::pbf::TagMap::of(pairs.iter().copied())
}

/// cat, body colour, topmark, attribute string, label and the sectors.
fn point(ty: &str, t: &teasi::pbf::TagMap) -> (u32, u32, u32, String, String, usize) {
    use teasi::osmpoint::*;
    let ls = lights(t);
    (
        category(ty, t).unwrap(),
        body_colour(ty, t),
        topmark(t),
        attributes(ty, t, &ls),
        label(ty, t, &ls),
        sectors(&ls).len(),
    )
}

#[test]
fn seamark_buoy_matches_python() {
    let t = seamark(&[
        ("seamark:type", "buoy_lateral"),
        ("seamark:buoy_lateral:shape", "can"),
        ("seamark:buoy_lateral:colour", "red"),
        ("seamark:buoy_lateral:category", "port"),
        ("seamark:topmark:shape", "cylinder"),
        ("seamark:topmark:colour", "red"),
        ("seamark:light:character", "Fl"),
        ("seamark:light:colour", "red"),
        ("seamark:light:period", "3"),
        ("seamark:light:range", "4.5"),
        ("seamark:name", "Buoy 7"),
    ]);
    let (cat, col, top, attrs, label, nsec) = point("buoy_lateral", &t);
    assert_eq!((cat, col, top), (0x0A_0100, 0x190, 0x685));
    assert_eq!(attrs, "19Port-hand Lateral Buoy|24Fl.R.3s4M");
    assert_eq!(label, "5Fl.R.3s4M");
    assert_eq!(nsec, 1); // a light without a sector still produces an entry
}

#[test]
fn seamark_beacon_matches_python() {
    let t = seamark(&[
        ("seamark:type", "beacon_cardinal"),
        ("seamark:beacon_cardinal:shape", "tower"),
        ("seamark:beacon_cardinal:colour", "black;yellow"),
        ("seamark:beacon_cardinal:colour_pattern", "horizontal"),
        ("seamark:beacon_cardinal:category", "north"),
        ("seamark:topmark:shape", "2 cones up"),
        ("seamark:topmark:colour", "black"),
    ]);
    let (cat, col, top, attrs, label, nsec) = point("beacon_cardinal", &t);
    assert_eq!((cat, col, top), (0x0A_0201, 0x4120, 0x48D));
    assert_eq!(attrs, "19North Cardinal Beacon");
    assert_eq!((label.as_str(), nsec), ("", 0));
}

#[test]
fn seamark_light_sectors_match_python() {
    let t = seamark(&[
        ("seamark:type", "light_major"),
        ("seamark:light:1:colour", "white"),
        ("seamark:light:1:character", "Oc"),
        ("seamark:light:1:period", "12"),
        ("seamark:light:1:sector_start", "30"),
        ("seamark:light:1:sector_end", "120"),
        ("seamark:light:1:range", "12"),
        ("seamark:light:2:colour", "red"),
        ("seamark:light:2:character", "Oc"),
        ("seamark:light:2:period", "12"),
        ("seamark:light:2:sector_start", "120"),
        ("seamark:light:2:sector_end", "200"),
        ("seamark:light:2:range", "9"),
        ("seamark:light:category", "floodlight"),
        ("seamark:fog_signal:category", "horn"),
        ("seamark:fog_signal:period", "30.0"),
        ("seamark:radar_transponder:category", "racon"),
        ("seamark:radar_transponder:group", "T"),
    ]);
    let (cat, _, _, attrs, label, _) = point("light_major", &t);
    assert_eq!(cat, 0x09_0003);
    assert_eq!(attrs, "19Flood light major|21Racon(T)|23Horn30s|24Oc.WR.12s9-12M");
    assert_eq!(label, "0Horn30s|0Racon(T)|5Oc.WR.12s9-12M");
    let secs = teasi::osmpoint::sectors(&teasi::osmpoint::lights(&t));
    assert_eq!(secs.len(), 2);
    assert_eq!((secs[0].0, secs[0].1, secs[0].2, secs[0].3), (1, 238.9, 341.3, 370));
    assert_eq!(secs[0].4, "Oc.W.12s");
    assert_eq!((secs[1].1, secs[1].2), (341.3, 22.7)); // wraps past north
}

#[test]
fn seamark_texts_match_python() {
    // several categories at once: unknown combination falls back to 3
    let t = seamark(&[
        ("seamark:type", "harbour"),
        ("seamark:harbour:category", "marina;fishing"),
        ("seamark:name", "Esbjerg"),
        ("phone", "+45 1"),
        ("website", "https://x"),
    ]);
    let (cat, _, _, attrs, _, _) = point("harbour", &t);
    assert_eq!(cat, 0x09_0306);
    assert_eq!(attrs, "01https://x|02+45 1|19- Yacht harbour/marina\n- Fishing harbour\n");

    let t = seamark(&[("seamark:type", "rock")]);
    assert_eq!(point("rock", &t).3, "19Rock|45submerged");

    let t = seamark(&[
        ("seamark:type", "signal_station_traffic"),
        ("seamark:signal_station_traffic:category", "port_control"),
    ]);
    let (cat, _, _, attrs, label, _) = point("signal_station_traffic", &t);
    assert_eq!(cat, 0x0C_000F);
    assert_eq!(attrs, "19Traffic Signal Station: Port Control");
    assert_eq!(label, "(Port Control)");
}

#[test]
fn osmpoint_record_round_trip() {
    use teasi::layers::{build_osmpoint, parse_osmpoint, u16enc, PointObj, PointRec, Sector};
    let label = u16enc("Oc.W.12s");
    let obj = PointObj {
        s: vec![0, 7, 0x0A_0100, 0x1234, 0x190, 0x685, 0, 0, 0, 0, 1, 0],
        name: u16enc("Buoy 7"),
        attrs: Vec::new(),
        label: Vec::new(),
        sectors: vec![Sector {
            w: vec![1, 2389, 3413, 370],
            a: 0,
            len: label.len() as u32,
            label,
        }],
    };
    let rec = PointRec { hdr: vec![0x30, 19, 0, 1, 0], objs: vec![obj] };
    let raw = build_osmpoint(&rec);
    assert_eq!(parse_osmpoint(&raw).unwrap(), rec);
}

// --------------------------------------------------------------------------
// GEOS (osmarea); skipped when libgeos cannot be loaded
// --------------------------------------------------------------------------

fn geos_here() -> bool {
    match teasi::geos::available() {
        Ok(()) => true,
        Err(e) => {
            eprintln!("skipped: {}", e);
            false
        }
    }
}

/// The values come from shapely 2.1.2 / GEOS 3.13.1 (see rust/README.md).
#[test]
fn geos_basics_match_shapely() {
    if !geos_here() {
        return;
    }
    use teasi::geos::Geom;
    // shapely.box builds its ring starting at (x1, y0), counter-clockwise
    let b = Geom::rect(10.0, 20.0, 30.0, 40.0).unwrap();
    assert_eq!(
        b.exterior(),
        vec![(30.0, 20.0), (30.0, 40.0), (10.0, 40.0), (10.0, 20.0), (30.0, 20.0)]
    );
    assert_eq!(b.bounds(), (10.0, 20.0, 30.0, 40.0));
    assert_eq!(b.area(), 400.0);
    // two squares sharing an edge merge into one rectangle
    let s1 = Geom::rect(0.0, 0.0, 10.0, 10.0).unwrap();
    let s2 = Geom::rect(10.0, 0.0, 20.0, 10.0).unwrap();
    let u = s1.union(&s2).unwrap();
    assert_eq!(u.type_id(), 3);
    assert_eq!(u.area(), 200.0);
    let all = teasi::geos::union_all(&[s1.clone_geom().unwrap(), s2.clone_geom().unwrap()]).unwrap();
    assert_eq!(all.area(), 200.0);
    // a square with a hole: difference keeps it as one polygon with one ring
    let hole = Geom::rect(2.0, 2.0, 4.0, 4.0).unwrap();
    let d = s1.difference(&hole).unwrap();
    assert_eq!(d.interiors().len(), 1);
    assert_eq!(d.area(), 96.0);
    // symmetric difference, clip and simplify
    assert_eq!(s1.sym_difference(&hole).unwrap().area(), 96.0);
    assert_eq!(s1.clip_by_rect(0.0, 0.0, 5.0, 10.0).unwrap().area(), 50.0);
    let zig = Geom::polygon(
        &[(0.0, 0.0), (10.0, 0.0), (10.0, 1.0), (5.0, 1.001), (0.0, 1.0), (0.0, 0.0)],
        &[],
    )
    .unwrap();
    // shapely: simplify(4.0) leaves a triangle, simplify(0.0005) keeps all points
    assert_eq!(
        zig.simplify(4.0).unwrap().exterior(),
        vec![(0.0, 1.0), (10.0, 0.0), (10.0, 1.0), (0.0, 1.0)]
    );
    assert_eq!(zig.simplify(0.0005).unwrap().exterior().len(), 6);
    assert!(s1.contains(&hole) && s1.intersects(&s2));
    assert!(Geom::empty().unwrap().is_empty());
}

/// shapely's STRtree reports in tree order, not sorted; the order decides which
/// polygon enters a union first, so it has to be the same.
#[test]
fn geos_strtree_order_matches_shapely() {
    if !geos_here() {
        return;
    }
    use teasi::geos::{Geom, Tree};
    let order = [7, 3, 9, 0, 5, 1, 8, 2, 6, 4];
    let gs: Vec<Geom> = order
        .iter()
        .map(|&i| {
            let x = i as f64;
            Geom::polygon(&[(x, 0.0), (x + 1.0, 0.0), (x + 1.0, 1.0), (x, 1.0), (x, 0.0)], &[])
                .unwrap()
        })
        .collect();
    let tree = Tree::new(&gs).unwrap();
    let hit = tree.query(&Geom::rect(-1.0, -1.0, 11.0, 2.0).unwrap());
    assert_eq!(hit, vec![3, 5, 7, 1, 9, 4, 8, 0, 6, 2]);
}

/// The tables of tools/compile_osmarea.py's area_class, spot-checked.
#[test]
fn area_class_matches_python() {
    use teasi::area::ATags;
    use teasi::osmarea::area_class;
    let tags = |pairs: &[(&str, &str)]| {
        ATags::of(pairs.iter().map(|&(k, v)| (k, v))).0
    };
    assert_eq!(area_class(&tags(&[("landuse", "residential")])), Some(("c3", None)));
    assert_eq!(area_class(&tags(&[("landuse", "forest")])), Some(("c5", Some(6))));
    assert_eq!(area_class(&tags(&[("natural", "heath")])), Some(("c5", Some(6))));
    assert_eq!(area_class(&tags(&[("landuse", "basin")])), Some(("c6", None)));
    assert_eq!(area_class(&tags(&[("natural", "water")])), Some(("c6", None)));
    // natural=water + water=river is not in the original
    assert_eq!(area_class(&tags(&[("natural", "water"), ("water", "river")])), None);
    assert_eq!(area_class(&tags(&[("natural", "water"), ("water", "lake")])), Some(("c6", None)));
    // landuse wins, but an unknown landuse falls through to natural
    assert_eq!(
        area_class(&tags(&[("landuse", "forest"), ("natural", "water")])),
        Some(("c5", Some(6)))
    );
    assert_eq!(
        area_class(&tags(&[("landuse", "nature_reserve"), ("natural", "beach")])),
        Some(("c5", Some(9)))
    );
    assert_eq!(area_class(&tags(&[("leisure", "marina")])), Some(("c5", Some(5))));
    assert_eq!(area_class(&tags(&[("leisure", "nature_reserve")])), None);
    assert_eq!(area_class(&tags(&[("man_made", "breakwater")])), Some(("c5", Some(17))));
    assert_eq!(area_class(&tags(&[("waterway", "dock")])), Some(("c6", None)));
    assert_eq!(area_class(&tags(&[("amenity", "parking")])), None);
    // area=no forbids an area, and KEYS decides whether it is kept at all
    assert!(ATags::of([("area", "no"), ("landuse", "forest")].into_iter()).0.no_area);
    assert!(ATags::of([("landuse", "forest")].into_iter()).1);
    assert!(!ATags::of([("building", "yes")].into_iter()).1);
}

/// An osmarea C record built from objects must parse back to them.
#[test]
fn osmarea_record_round_trip() {
    use std::collections::BTreeMap;
    use teasi::layers::{self, Var, C_SPEC};
    use teasi::osmarea::{make_object, record, CELL};
    let cell = (100i64, 200i64);
    let ring = |n: u32| {
        (
            n, // detail level
            vec![
                (cell.0 * CELL + 10, cell.1 * CELL + 20),
                (cell.0 * CELL + 30, cell.1 * CELL + 20),
                (cell.0 * CELL + 30, cell.1 * CELL + 40),
                (cell.0 * CELL + 10, cell.1 * CELL + 20),
            ],
        )
    };
    let mut objs: BTreeMap<&str, Vec<layers::Item>> = BTreeMap::new();
    objs.insert("c3", vec![make_object(cell, "", None, &[ring(12)], "c3")]);
    objs.insert("c5", vec![make_object(cell, "", Some(8), &[ring(14), ring(9)], "c5")]);
    objs.insert("c6", vec![make_object(cell, "#OW", None, &[ring(9)], "c6")]);
    let raw = record(cell, &objs);
    let rec = layers::parse_c(&raw).unwrap();
    assert_eq!(layers::build_arrays(&rec, &C_SPEC), raw);
    assert_eq!(rec.hdr[1], cell.1 as u32);
    assert_eq!((rec.hdr[7], rec.hdr[11], rec.hdr[13]), (1, 1, 1));
    let c5 = &rec.get("c5", &C_SPEC)[0];
    assert_eq!(c5.s[2], 8); // the class
    assert_eq!(c5.s[3], 20 << 16 | 10); // low corner, relative to the cell
    assert_eq!(c5.s[4], 40 << 16 | 30); // high corner
    let Var::U32(geom) = &c5.v[1] else { panic!("geometry") };
    let parts = layers::geometry_parts(geom);
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].0, 14); // detail level of the first ring
    assert_eq!(parts[0].1.len(), 4);
    assert_eq!(parts[1].0, 9);
    let Var::U16(name) = &rec.get("c6", &C_SPEC)[0].v[0] else { panic!("name") };
    assert_eq!(layers::u16str(name), "#OW");
}

// --------------------------------------------------------------------------
// the osm layer: tag tables, geometry and the name table
// --------------------------------------------------------------------------

fn wtags(pairs: &[(&str, &str)]) -> teasi::pbf::TagMap {
    teasi::pbf::TagMap::of(pairs.iter().copied())
}

/// Values from tools/compile_osm.py: road_class, flags with and without route
/// relations, passable and category.
#[test]
fn road_attributes_match_python() {
    use teasi::osm::{flags, road_class};
    let rels: Vec<teasi::way::Route> = vec![
        ("bicycle".into(), Some("lcn".into()), None),
        ("hiking".into(), None, Some("E1".into())),
        ("mtb".into(), Some("icn".into()), None),
    ];
    let cases: [(&[(&str, &str)], Option<u32>, u32, u32); 6] = [
        (
            &[("highway", "residential"), ("surface", "asphalt"), ("lanes", "2"), ("oneway", "yes")],
            Some(7),
            134498946,
            19072,
        ),
        (
            &[("highway", "cycleway"), ("bicycle", "designated"), ("mtb:scale", "1")],
            Some(10),
            134481826,
            1952,
        ),
        (
            &[("highway", "track"), ("surface", "sett"), ("access", "private"), ("foot", "permissive")],
            Some(12),
            2281964306,
            2147484432,
        ),
        (
            &[("highway", "footway"), ("cycleway", "lane"), ("bicycle", "dismount"), ("lanes", "x")],
            Some(11),
            1745094530,
            1610614656,
        ),
        (&[("route", "ferry")], Some(9), 134480770, 896),
        (
            &[("highway", "primary"), ("junction", "roundabout"), ("name:de", "X"), ("name", "Y")],
            Some(2),
            134482819,
            2945,
        ),
    ];
    for (pairs, cls, with, without) in cases {
        let t = wtags(pairs);
        assert_eq!(road_class(&t), cls, "class of {:?}", pairs);
        assert_eq!(flags(&t, &rels), with, "flags of {:?}", pairs);
        assert_eq!(flags(&t, &[]), without, "flags without routes of {:?}", pairs);
    }
    // area=yes is not a road, a ferry with a highway tag goes by the highway
    assert_eq!(road_class(&wtags(&[("highway", "primary"), ("area", "yes")])), None);
    assert_eq!(road_class(&wtags(&[("route", "ferry"), ("highway", "service")])), Some(7));
}

#[test]
fn line_types_match_python() {
    use teasi::osm::{line_type, LineType};
    let a3 = |t: &[(&str, &str)]| line_type(&wtags(t));
    assert_eq!(a3(&[("railway", "tram")]), Some(LineType::A3(0)));
    assert_eq!(a3(&[("man_made", "pier")]), Some(LineType::A3(4)));
    assert_eq!(a3(&[("power", "line")]), Some(LineType::A3(10)));
    assert_eq!(a3(&[("waterway", "ditch")]), None); // only when named
    assert_eq!(
        a3(&[("waterway", "ditch"), ("name", "Gr\u{f8}ften")]),
        Some(LineType::A4(7, "Gr\u{f8}ften".into()))
    );
    assert_eq!(
        a3(&[("waterway", "river"), ("name", "Guden\u{e5}"), ("name:de", "Guden"), ("name:en", "Guden")]),
        Some(LineType::A4(7, "[DANGuden\u{e5}\u{a6}GERGuden\u{a6}ENGGuden]".into()))
    );
    // the orientation is rounded half to even and prefixed to the label
    assert_eq!(
        a3(&[("seamark:type", "navigation_line"), ("seamark:navigation_line:orientation", "12.5")]),
        Some(LineType::A4(8, "12(leading)".into()))
    );
    assert_eq!(
        a3(&[("seamark:type", "recommended_track"), ("seamark:recommended_track:orientation", "x")]),
        Some(LineType::A4(9, "(fixed_marks)".into()))
    );
    assert_eq!(
        a3(&[("seamark:type", "recommended_track")]),
        Some(LineType::A4(9, "(fixed_marks)".into()))
    );
}

#[test]
fn clipping_and_packing_match_python() {
    let pts = [(0.0, 0.0), (100.0, 50.0), (200.0, -20.0)];
    let parts = teasi::osm::clip(&pts, 10.0, -10.0, 150.0, 40.0);
    assert_eq!(
        parts,
        vec![
            vec![(10.0, 5.0), (80.0, 40.0)],
            vec![(114.28571428571428, 40.0), (150.0, 15.0)],
        ]
    );
    // repeated points are dropped, the coordinates are rounded half to even
    let geo = teasi::osm::encode_parts(
        &[vec![(100.4, 200.5), (100.4, 200.5), (300.5, 400.5)]],
        0.0,
        0.0,
    );
    assert_eq!(geo, vec![131074, 46662244, 59769644]);
    // the bbox of a line near a cell corner offers four cells ...
    assert_eq!(
        teasi::osm::cells_of(32000.0, 33000.0, 65000.0, 70000.0),
        vec![(0, 1), (0, 2), (1, 1), (1, 2)]
    );
    // ... but (1, 1) stays empty after the clip, so only three carry geometry
    let cells: Vec<(i64, i64)> = teasi::osm::place(&[(32000.0, 65000.0), (33000.0, 70000.0)])
        .into_iter()
        .map(|(c, _)| c)
        .collect();
    assert_eq!(cells, vec![(0, 1), (0, 2), (1, 2)]);
}

#[test]
fn name_table_matches_python() {
    // casefold, Danish letters spelled out, accents dropped
    for (name, key) in [
        ("\u{c5}gade", "agade"),
        ("\u{c6}blevej", "aeblevej"),
        ("\u{d8}stergade", "ostergade"),
        ("Stra\u{df}e", "strasse"),
        ("\u{e9}cole", "ecole"),
        ("\u{c6}r\u{f8}vej", "aerovej"),
    ] {
        assert_eq!(teasi::osm::sort_key(name), key, "sort key of {}", name);
    }
    let names: Vec<String> =
        ["\u{d8}ster All\u{e9}", "Aagade", "\u{d8}ster Vej"].iter().map(|s| s.to_string()).collect();
    let (blk5, blk6, index) = teasi::osm::name_table(&names);
    assert_eq!(
        blk5,
        vec![0, 0, 0, 1, 14, 0, 0, 2, 26, 0, 0, 0, 14, 0, 0, 2, 36, 0, 0, 0]
    );
    // the words are UTF-16LE with a length but no NUL, and shared between names
    assert_eq!(&blk6[..4], &[6, 0, b'A', 0]);
    assert_eq!(blk6.len(), 44);
    assert_eq!(index["Aagade"], 0);
    assert_eq!(index["\u{d8}ster All\u{e9}"], 4);
    assert_eq!(index["\u{d8}ster Vej"], 12);
}

// --------------------------------------------------------------------------
// the ta layer: house numbers, postcodes, cell cutting and the index keys
// --------------------------------------------------------------------------

/// Values from tools/compile_ta.py.
#[test]
fn house_numbers_match_python() {
    use teasi::ta::house_numbers;
    let cases: [(&str, &[i64]); 13] = [
        ("12", &[12]),
        (" 12a ", &[12]),
        ("12-16", &[12, 16]),
        ("12a - 16b", &[12, 16]),
        ("7/9", &[7, 9]),
        ("Flat 3", &[]),
        ("", &[]),
        ("12abc", &[]),
        ("0", &[0]),
        ("1A", &[1]),
        ("12 \u{2013} 14", &[12, 14]),
        ("99999", &[99999]),
        ("12b", &[12]),
    ];
    for (s, want) in cases {
        assert_eq!(house_numbers(s), want, "house numbers of {:?}", s);
    }
}

#[test]
fn outward_codes_match_python() {
    use teasi::ta::outward;
    let cases: [(&str, Option<&str>); 10] = [
        ("SW1A 1AA", Some("SW1A")),
        ("sw1a1aa", Some("SW1A")),
        ("E1 3AB", Some("E1")),
        ("E12 3AB", Some("E12")),
        ("A1 2BC", Some("A1")),
        ("AB12 3CD", Some("AB12")),
        ("ABC1 2DE", None),
        ("SW1A", None),
        ("12345", None),
        ("N1 9GU", Some("N1")),
    ];
    for (s, want) in cases {
        assert_eq!(outward(s).as_deref(), want, "outward code of {:?}", s);
    }
}

#[test]
fn split_cells_matches_python() {
    let r = teasi::ta::split_cells(&[(32700.0, 100.0), (33000.0, 200.0), (65600.0, 300.0)]);
    let cells: Vec<(i64, i64)> = r.iter().map(|(c, _)| *c).collect();
    assert_eq!(cells, vec![(0, 0), (1, 0), (2, 0)]);
    assert_eq!(r[0].1.len(), 2);
    assert_eq!(r[1].1.len(), 3);
    // the crossing point is shared by both parts
    assert_eq!(r[0].1[1], r[1].1[0]);
    assert_eq!(r[1].1[2], r[2].1[0]);
    assert!((r[0].1[1].1 - 122.666667).abs() < 1e-6);
    assert!((r[1].1[2].1 - 299.803681).abs() < 1e-6);
}

#[test]
fn side_ranges_match_python() {
    use teasi::ta::side_range;
    let cases: [(&[(f64, i64)], u32); 7] = [
        (&[], 0xFFFF_7FFF),
        (&[(0.1, 12)], 0xC000C),
        (&[(0.1, 12), (0.9, 24)], 0x18000C),
        (&[(0.9, 12), (0.1, 24)], 0xC0018),
        // one odd number among three even ones is more than 10 %: bit 15
        (&[(0.1, 2), (0.5, 4), (0.9, 6), (0.3, 7)], 0x78002),
        (&[(0.1, 2), (0.5, 4), (0.9, 6), (0.3, 7), (0.4, 9), (0.6, 11)], 0xB8002),
        // all at the same position: ascending
        (&[(0.5, 10), (0.5, 20)], 0x14000A),
    ];
    for (nums, want) in cases {
        assert_eq!(side_range(&mut nums.to_vec()), want, "side range of {:?}", nums);
    }
}

#[test]
fn index_keys_match_python() {
    assert_eq!(teasi::ta_index::fold("K\u{f8}benhavn"), "kobenhavn");
    assert_eq!(teasi::ta_index::fold("\u{c6}r\u{f8}skobing"), "aroskobing");
    assert_eq!(teasi::ta_index::fold("Stra\u{df}e"), "strasse");
    assert_eq!(teasi::ta_index::fold("\u{c5}RHUS"), "arhus");
    assert_eq!(teasi::ta_index::fold("Mal\u{f6}"), "malo");
    assert_eq!(teasi::ta::keys("K\u{f8}benhavn, N\u{f8}rrebro"), ["kobenhavn", "norrebro"]);
    assert_eq!(teasi::ta::keys("King's Lynn"), ["kings", "kings lynn", "lynn"]);
    assert_eq!(
        teasi::ta::keys("Stoke-on-Trent"),
        ["on", "stoke", "stoke on trent", "trent"]
    );
    assert_eq!(teasi::ta::keys("SW1A"), ["sw1a"]);
    assert_eq!(teasi::ta::keys("\u{c5}rhus"), ["arhus"]);
}
