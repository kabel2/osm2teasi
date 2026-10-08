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
        let (x, y) = teasi::osm::xy_dm(dm_lon, dm_lat);
        assert_eq!(x.to_bits(), bx, "x for {} {}", dm_lon, dm_lat);
        assert_eq!(y.to_bits(), by, "y for {} {}", dm_lon, dm_lat);
    }
    assert_eq!(teasi::osm::SCALE.to_bits(), 0x4126c16c16c16c17);
}

#[test]
fn node_index_keeps_what_it_was_asked_for() {
    let mut ix = teasi::osm::NodeIndex::new(vec![7, 3, 7, 1]);
    assert_eq!(ix.len(), 3);
    assert!(ix.wants(3) && !ix.wants(4));
    assert_eq!(ix.missing(), 3);
    ix.set(3, 102345678, 553456789);
    ix.set(4, 0, 0); // not asked for: ignored
    assert_eq!(ix.missing(), 2);
    assert_eq!(ix.get(3), Some(teasi::osm::xy_dm(102345678, 553456789)));
    assert_eq!(ix.get(7), None);
}

#[test]
fn fsum_matches_pythons_sum() {
    // CPython's sum() compensates since 3.12; a naive loop gives 0.0 and
    // 0.9999999999999999 here
    assert_eq!(teasi::osm::fsum([1.0, 1e16, 1.0, -1e16].into_iter()), 2.0);
    assert_eq!(teasi::osm::fsum(std::iter::repeat_n(0.1, 10)).to_bits(), 1.0f64.to_bits());
}

#[test]
fn rings_are_joined_at_shared_ends() {
    use teasi::osm::assemble_rings;
    // two ways forming one ring, the second one reversed
    let ways = vec![vec![1, 2, 3], vec![5, 4, 3], vec![1, 5]];
    assert_eq!(assemble_rings(&ways), Some(vec![vec![1, 2, 3, 4, 5]]));
    // a closed way is a ring of its own, minus the repeated end
    assert_eq!(assemble_rings(&[vec![1, 2, 3, 1]]), Some(vec![vec![1, 2, 3]]));
    // two independent rings
    assert_eq!(
        assemble_rings(&[vec![1, 2, 1], vec![7, 8, 7]]),
        Some(vec![vec![1, 2], vec![7, 8]])
    );
    assert_eq!(assemble_rings(&[vec![1, 2, 3]]), None); // stays open
}

#[test]
fn an_inner_ring_does_not_count_towards_the_centre() {
    use teasi::osm::area_centre;
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
    let ix = teasi::osm::NodeIndex::new(ids.clone());
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
    let empty = teasi::osm::NodeIndex::new(vec![]);
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
