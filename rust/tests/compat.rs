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
