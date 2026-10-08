//! Compatibility with the Python tools.  The reference values come from
//! tools/pc1.py and tools/chart.py; the full test is `teasi check <chart>`
//! over the original files, which needs chart data and therefore lives in the
//! CLI rather than here.

use md5::{Digest, Md5};
use teasi::{chart, layers, lzma, pc1, ta_index};

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
fn ring_centre_drops_the_repeated_point() {
    let pts = [(1.0, 10.0), (2.0, 20.0), (3.0, 30.0), (1.0, 10.0)];
    assert_eq!(teasi::osm::ring_centre(&pts), Some((2.0, 20.0)));
    assert_eq!(teasi::osm::ring_centre(&pts[..3]), Some((2.0, 20.0)));
    assert_eq!(teasi::osm::ring_centre(&[(5.0, 6.0)]), Some((5.0, 6.0)));
    assert_eq!(teasi::osm::ring_centre(&[]), None);
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
