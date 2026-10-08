//! Fixtures for the pieces that have to agree bit for bit with something
//! outside this crate.  The expected values were taken, while porting, from the
//! firmware's own output, from the Python reference implementation this port
//! replaces, and from Pillow 12.3 / numpy 2.5; they are frozen here.  The full
//! test is `teasi check <chart>` over the original files, which needs chart
//! data and therefore lives in the CLI rather than here.

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
    // reference values from the Python side (SCALE = 2**28 / 360)
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
    let ix = teasi::pbf::NodeIndex::new(vec![7, 3, 7, 1]);
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
// osmpoi
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
// osmpoint; all expected values printed by the Python side
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

/// The area_class tables, spot-checked.
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

/// Reference values: road_class, flags with and without route
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

/// Reference values for the address layer.
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
fn postcode_keys() {
    use teasi::ta::postcode_key;
    let cases: [(&str, Option<&str>); 9] = [
        ("SW1A 1AA", Some("SW1A")),
        ("SW1A", Some("SW1A")),
        ("46325", Some("46325")),
        (" 46325 ", Some("46325")),
        ("1234 AB", Some("1234")),
        ("K1A 0B1", Some("K1A")),
        ("00-950", Some("00-950")),
        ("London", None),
        ("46325;46326", None),
    ];
    for (s, want) in cases {
        assert_eq!(postcode_key(s).as_deref(), want, "postcode key of {:?}", s);
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
    // the region behind the name is not a key
    assert_eq!(teasi::ta::keys("Borken (Nordrhein-Westfalen)"), ["borken"]);
    assert_eq!(teasi::ta::keys("Bocholt, Biemenhorst (Nordrhein-Westfalen)"), ["biemenhorst", "bocholt"]);
}

/// A tiny generator the Python reference shared, so that the shapes in the
/// next test are the ones PIL drew when the md5s below were taken.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[test]
fn polygon_fill_matches_pil() {
    use teasi::raster::Mask;
    let mut r = Lcg(12345);
    let mut h = Md5::new();
    for _ in 0..200 {
        let k = 3 + (r.next() * 7.0) as usize;
        let pts: Vec<(f64, f64)> =
            (0..k).map(|_| (-6.0 + r.next() * 44.0, -6.0 + r.next() * 44.0)).collect();
        let mut m = Mask::new(32, 32, 0);
        m.polygon(&pts, 1);
        h.update(&m.px);
    }
    assert_eq!(format!("{:x}", h.finalize()), "376aa0b5f07a32f40bcc33a80221420a");
}

#[test]
fn lanczos_resize_matches_pil() {
    use teasi::raster::{resize, Rgb};
    let mut img = Rgb::new(64, 64);
    for x in 0..64usize {
        for y in 0..64usize {
            img.set(
                x,
                y,
                (
                    ((x * 37 + y * 17) % 251) as u8,
                    ((x * 11 + y * 53) % 241) as u8,
                    ((x * 73 + y * 29) % 233) as u8,
                ),
            );
        }
    }
    for (n, want) in [
        (32usize, "d1c5f82006c819a0aaf38287eb9d9f22"),
        (16, "b046428afcdeeded3d5798cdd5a38b1e"),
        (8, "c38c68a7311f264e70c52349d1649cf0"),
    ] {
        let out = resize(&img, n, n);
        assert_eq!(format!("{:x}", Md5::digest(out.rgb())), want, "resize to {}", n);
    }
}

#[test]
fn shading_matches_numpy() {
    let side = 18usize;
    let mut h = vec![0.0f64; side * side];
    for i in 0..side {
        for j in 0..side {
            h[i * side + j] = ((i * 7 + j * 13) % 23) as f64 * 3.5 - ((i * j) % 5) as f64;
        }
    }
    let s = teasi::terrain::shade(&h, side, 55.0);
    let mut m = Md5::new();
    for v in &s {
        m.update(v.to_le_bytes());
    }
    assert_eq!(format!("{:x}", m.finalize()), "9a66a7403e4c2a3f58441f68950b6916");
}

// --------------------------------------------------------------------------
// dem (the Copernicus elevation grid)
// --------------------------------------------------------------------------

/// A 20x13 float32 GeoTIFF as tifffile writes one: tiled 16x16 (so two tiles,
/// the second one mostly padding), DEFLATE, floating-point predictor -- the
/// shape the Copernicus tiles come in.  Its values are `Lcg` with seed 99.
const DEM_TIFF: &str = "\
49492a0008000000110000010400010000000d0000000101040001000000140000000201030001\
000000200000000301030001000000b28000000601030001000000010000000e01020014000000\
da0000001501030001000000010000001a01050001000000fe0000001b01050001000000060100\
00280103000100000001000000310102000c0000000e0100003d01030001000000030000004201\
0400010000001000000043010400010000001000000044010400020000001a0100004501030002\
000000d3031101530103000100000003000000000000007b227368617065223a205b32302c2031\
335d7d000000000000000000000000000000000001000000010000000100000001000000746966\
6666696c652e70790030010000030500000000000000000000000000000000789c25937b505465\
18c69fb32297e2a25c422e6de3e0b2c972274b2e060e2223217129248135cc61c801350c82d03d\
e7ac5c449052b9bad0344c126008443241480884356d20b0b166db1481132dd872190463dde377\
d6ef8fefbfe7fd3deffbbcef204b8166691680dea40fd83c3e54e134575072cc66f67ce1696041\
2794687cccac4c72cba7273701d637e4558d914b56bde39e837fcf03fb88d0a8e63fa2afbefe8e\
da669370b2d52bf37ecb0890f7782658acd37d7029a4b9f3c122405d6e9174984dad171dcca67a\
5e0086089b63c072e004f41ee0f3154b0bd794f22bdbf5fd2979b6c0aaada849e4b437dcd2cace\
7759082c16a7754f7db7e4159355aaec5b277c8ea2cf0a288aa30c2c730ba8595f3ddf60504b5c\
9acf74fd540f8c047fb373463c1e23b039eeeefb32e0f8de98fb83a2e7fe4a686cadeb8f04c229\
c2a7685ac05190bd0e5c2bbf7c473a2ae975fa756d67f56d20782d215aee2d7d525be0d6ebb615\
88f7983d5219a7fcdebac92df0b50bbc9e770e03055288f49fec2e5e14ddd725db516dceed5540\
b6e24b6783acd63cd4ecea277211208e4cb7eeb2adf5fdc379e06793e779ff0c180ae4110344ff\
466645fbc2c03f1f7f3431587f420be46e78344f773f4abc1011a0939601165e3f362c375d6c68\
bb6d72a9d8999f3fd1c120a0398ae5e7d7ce8cd5e435fae4f88d5e55160e03ecbb87b43d9ae35a\
2688528b4380b2fe589534599a149df06adf2937e29f61085a6020bf9ce7ff22b31bf8cd5f6efe\
d58742857926f0ffd0d6f09631cbc9436d75db1e3d24f5d263d60266df5cf04c4d53447ccbf369\
0e8c00345863ff671d84c13f7897ec35750fcfb7789fe4fdd6545ac6ca46eacc67fe5dd7ba8180\
8bee0505c2be3445dc7ef1b092cf5f46c02401163296e4676a37faefc1ec93e726926e7eaac801\
7625766e8950b97a1ef52c2b394996ada2b24339122868b2f23930d9d1c5fb276032418e7a2207\
e9bf14ce57a25cb52ebb0a0afdee040059cd45b339dbe3e35db487635b75c080bdde21e35c7aad\
fdd4e3bbe3378cfb2ba3588e869ce6e4c47fb3e677372f55ee7e3b7d9cc0210ac8b0094bd5e803\
a33d6a065ed941fc476737481c57aa8ffe59dc5f37a77a963f1f00cbb10cdd0f1c0ef1ee1996fc\
67db9abbf9661ce9bf5ee4e1b8a5e7bafacc34b3bbca02d06c6c0bd0e64a2c828441f3abf78c7c\
9a35088ce763dcbf41a73d778595f6212f4d503eb180b2f4ebc825e9a99ece447559d631c032b4\
ebc4749bff8e87cbaab87b84bf8fa6c901530c456a70647e2da75b94a161555fc404ef36993703\
5e7c5b94728b1934457e896ffe11c02f2a3ee940755853a97f7af69c02780ab35e9210789c3bdc\
c8f09fb1be819181e13fc3ff7d0c0c1bd2421af73bf3ef9b146bba3d9e8781c1d3eee7e3b34e2f\
62736ebfee39bf878121614bab64587385cfd6ea6865d67f0c0c0799181aeb1a191a1afeff67fe\
07d4df3f237d79ebd3a8b2236d27a4c5fc1818be372d4fe9397a87293f2ca9e3e05d06068173ef\
c3f416b4451f8ab6b9d750cec0e0c20004ff1918eb1beb1b1980e667ac657fba46ee82f4cf7fa1\
53f2fe3030546db4fccdccf37be9ffd30dcf8b81e6b9dd96f81d94be7ebbff951f8cd777313038\
37343434fe67fcd7d8083407a8bf533cf4b72bbb52ddb7fb874cdb2f3230bcb8c0d4a3b2c8a0cd\
7075dcd499d718180ae648662c48659bfc3824fd299707c3281805231a0000a5f46e01";

#[test]
fn gaussian_kernel_matches_scipy() {
    // `scipy.ndimage._filters._gaussian_kernel1d(sigma, 0, int(4*sigma+0.5))`,
    // md5 over the little-endian f64 weights.  sigma 33 has 265 of them, which
    // is where numpy's pairwise summation starts splitting the normaliser.
    for (sigma, n, want) in [
        (1.0, 9, "137a5f3584d37a01893d3d7f4e12d607"),
        (2.5, 21, "9424ec249627a548355d90564fceb2cc"),
        (33.0, 265, "6285e5746915b53dca6b542a92d210fd"),
    ] {
        let (lw, w) = teasi::dem::kernel(sigma);
        assert_eq!(w.len(), n, "sigma {}", sigma);
        assert_eq!(2 * lw + 1, n);
        let mut m = Md5::new();
        for v in &w {
            m.update(v.to_le_bytes());
        }
        assert_eq!(format!("{:x}", m.finalize()), want, "sigma {}", sigma);
    }
}

#[test]
fn gaussian_filter_matches_scipy() {
    // `scipy.ndimage.gaussian_filter(a, sigma)` over the same 37x41 f32 grid.
    let (rows, cols) = (37usize, 41usize);
    let mut r = Lcg(12345);
    let a: Vec<f32> = (0..rows * cols).map(|_| (r.next() * 900.0 - 100.0) as f32).collect();
    let mut m = Md5::new();
    for v in &a {
        m.update(v.to_le_bytes());
    }
    assert_eq!(format!("{:x}", m.finalize()), "757a14f3e0c94cf2b5dd42bc1a7adb60", "input");
    for (sigma, want) in
        [(1.0, "daeacdf089fb959e18ef475bd2c53d3e"), (2.5, "2a7c97530536faaf720808476e146e20")]
    {
        let out = teasi::dem::gaussian(&a, rows, cols, sigma);
        let mut m = Md5::new();
        for v in &out {
            m.update(v.to_le_bytes());
        }
        assert_eq!(format!("{:x}", m.finalize()), want, "sigma {}", sigma);
    }
}

#[test]
fn tiled_smoothing_matches_the_full_grid() {
    // 4x5 tiles of 12x12, some not stored (0, like open sea): every stored
    // tile must come out as the same floats `gaussian` gives the full grid,
    // and the file must read back unchanged.
    use teasi::heights::{Heights, NONE};
    let (t, trows, tcols) = (12usize, 4usize, 5usize);
    let (rows, cols) = (t * trows, t * tcols);
    let stored = |tr: usize, tc: usize| (tr * 7 + tc * 3) % 4 != 0;
    let mut r = Lcg(777);
    let mut full = vec![0f32; rows * cols];
    let mut slots = vec![NONE; trows * tcols];
    let mut data = Vec::new();
    for tr in 0..trows {
        for tc in 0..tcols {
            if !stored(tr, tc) {
                continue;
            }
            slots[tr * tcols + tc] = (data.len() / (t * t)) as u32;
            for i in 0..t {
                for j in 0..t {
                    let v = (r.next() * 900.0 - 100.0).max(0.0) as f32;
                    full[(tr * t + i) * cols + tc * t + j] = v;
                    data.push(v);
                }
            }
        }
    }
    for sigma in [1.0, 2.5] {
        let want = teasi::dem::gaussian(&full, rows, cols, sigma);
        let mut h = Heights {
            rows,
            cols,
            lon0: 5.0,
            lat0: 50.0,
            step: 1.0 / t as f64,
            tile: t,
            slots: slots.clone(),
            data: data.clone(),
        };
        teasi::dem::smooth(&mut h, sigma);
        for tr in 0..trows {
            for tc in 0..tcols {
                for i in 0..t {
                    for j in 0..t {
                        let (rr, cc) = (tr * t + i, tc * t + j);
                        let w = if stored(tr, tc) { want[rr * cols + cc] } else { 0.0 };
                        assert_eq!(h.get(rr, cc).to_bits(), w.to_bits(), "sigma {} at {},{}", sigma, rr, cc);
                    }
                }
            }
        }
        let path = std::env::temp_dir().join(format!("teasi-heights-{}.bin", std::process::id()));
        h.write(path.to_str().unwrap()).unwrap();
        let back = Heights::load(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!((back.rows, back.cols, back.tile), (rows, cols, t));
        assert_eq!((back.lon0, back.lat0, back.step), (h.lon0, h.lat0, h.step));
        assert_eq!(back.slots, h.slots);
        assert!(back.data.iter().zip(&h.data).all(|(a, b)| a.to_bits() == b.to_bits()));
        assert!(back.covers(5.0 + 13.0 / 12.0, 50.0 - 1.0, 5.0 + 14.0 / 12.0, 50.0 - 0.9));
        assert!(!back.covers(0.0, 40.0, 1.0, 41.0));
    }
}

#[test]
fn dem_tiff_matches_tifffile() {
    let raw: Vec<u8> = (0..DEM_TIFF.len() / 2)
        .map(|i| u8::from_str_radix(&DEM_TIFF[2 * i..2 * i + 2], 16).unwrap())
        .collect();
    let (rows, cols, z) = teasi::dem::decode_tiff(&raw, "fixture").unwrap();
    assert_eq!((rows, cols), (20, 13));
    let mut r = Lcg(99);
    for (i, &v) in z.iter().enumerate() {
        assert_eq!(v, (r.next() * 2000.0 - 500.0) as f32, "value {}", i);
    }
}

/// A layer with nothing in the area comes out as a 120-byte header, and the
/// checksum covers 1 KB from 0x44 plus the last KB -- so signing it has to be
/// an error, not an index panic.  Andorra has no seamarks.
#[test]
fn signing_a_file_without_records_fails() {
    let s = chart::Signer::bound();
    assert!(s.mac(&[0u8; 120]).is_err());
    assert!(s.mac(&[0u8; 0x843]).is_err());
    assert!(s.mac(&[0u8; 0x844]).is_ok());
}

#[test]
fn regions_split_countries_the_firmware_knows_by_part() {
    let f = |id: &str, name: &str, parent: &str, path: &str| {
        format!(
            r#"{{"properties":{{"id":"{}","name":"{}","parent":"{}","urls":{{"pbf":"https://download.geofabrik.de/{}-latest.osm.pbf"}}}}}}"#,
            id, name, parent, path
        )
    };
    let feats = [
        f("canada", "Canada", "north-america", "north-america/canada"),
        f("yukon", "Yukon", "canada", "north-america/canada/yukon"),
        f("quebec", "Quebec", "canada", "north-america/canada/quebec"),
        f("us", "United States of America", "north-america", "north-america/us"),
        f("us/georgia", "Georgia", "north-america", "north-america/us/georgia"),
        f("georgia", "Georgia", "europe", "europe/georgia"),
        f("gcc-states", "GCC States", "asia", "asia/gcc-states"),
        f("bayern", "Bayern", "germany", "europe/germany/bayern"),
    ];
    let json = format!(r#"{{"features":[{}]}}"#, feats.join(","));
    let got: Vec<(String, u32)> = teasi::regions::regions(&json)
        .unwrap()
        .into_iter()
        .map(|r| (r.path, r.code))
        .collect();
    let code = |s: &str| teasi::chart::country(s).unwrap().0;
    assert_eq!(
        got,
        vec![
            ("asia/gcc-states".to_string(), code("Saudi Arabia")),
            ("europe/georgia".to_string(), code("Georgia")),
            ("north-america/canada/quebec".to_string(), code("Québec (Canada)")),
            ("north-america/canada/yukon".to_string(), code("Yukon Territory (Canada)")),
            ("north-america/us/georgia".to_string(), code("Georgia (United States)")),
        ]
    );
}

#[test]
fn grid_pairs_with_coincident_points() {
    // all ends of a street on one point (Ohio): the grid shrinks to 1e-9 and
    // pairs() must still stop at its edge
    let g = teasi::grid::Grid::new(&[(5.0, 7.0); 4]);
    assert_eq!(g.pairs(30.0), vec![(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)]);
    // and agree with every pair checked by hand elsewhere
    let mut r = Lcg(4242);
    let pts: Vec<(f64, f64)> = (0..300).map(|_| (r.next() * 100.0, r.next() * 3.0)).collect();
    let mut want = Vec::new();
    for i in 0..pts.len() {
        for j in i + 1..pts.len() {
            let (dx, dy) = (pts[j].0 - pts[i].0, pts[j].1 - pts[i].1);
            if dx * dx + dy * dy <= 2.5 * 2.5 {
                want.push((i as u32, j as u32));
            }
        }
    }
    assert_eq!(teasi::grid::Grid::new(&pts).pairs(2.5), want);
}
