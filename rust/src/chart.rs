//! The outer shell of a chart file: header, checksum, tile directory, records.
//! Port of tools/chart.py; the format is documented in
//! docs/KARTEN_ENTSCHLUESSELUNG.md sections 1 to 4.

use anyhow::{bail, Result};
use md5::{Digest, Md5};

use crate::lzma;
use crate::pc1;

pub const MAGIC: u32 = 0x1B62;
pub const SECRET: &[u8] = b"d8ethebrestezexaqathaTrepedEkubafr5vuhaprupe3ucUphedeyuhaGespenU";
pub const HEAD: usize = 0x159C; // tile head size; records start here
pub const BLOB: usize = 0x157C; // the tile's 32-byte record key, PC1 encrypted

/// bikenav.exe FUN_002050a0: device IDs with one of these prefixes use the
/// built-in key, every other one a key derived from the ID itself.
pub const STATIC_KEY: &[u8] = b"E89ACE5CE51E0669B4BA068CE8F63990";
const KNOWN_PREFIXES: [&[u8; 8]; 19] = [
    b"20130125", b"20130212", b"20130213", b"20130807", b"20131010", b"20131020", b"20131026",
    b"20150215", b"20160505", b"20160509", b"20161014", b"20161028", b"20170606", b"20170707",
    b"20180914", b"20181225", b"20190320", b"20190415", b"20190618",
];

/// Serial number the charts are bound to; override with TEASI_DEVICE.
pub fn device() -> Vec<u8> {
    std::env::var("TEASI_DEVICE")
        .unwrap_or_else(|_| "2013021200000368".to_string())
        .into_bytes()
}

/// Today as "YYYYMMDD", the default version date of a freshly built chart.
/// Days to a calendar date after Howard Hinnant's civil_from_days.
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before 1970")
        .as_secs();
    let z = (secs / 86400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{:04}{:02}{:02}", y, m, d)
}

pub fn global_key(device_id: &[u8]) -> Vec<u8> {
    let prefix = &device_id[..8.min(device_id.len())];
    if KNOWN_PREFIXES.iter().any(|p| p.as_slice() == prefix) {
        return STATIC_KEY.to_vec();
    }
    let digest = Md5::digest(prefix);
    format!("{:X}", digest).into_bytes()
}

/// Header MAC: generic with an empty device, otherwise bound to that device.
pub fn header_md5(d: &[u8], device: &[u8]) -> [u8; 16] {
    let mut h = Md5::new();
    h.update(&d[4..0x34]);
    h.update(SECRET);
    h.update(&d[0x44..0x444]);
    h.update(&d[d.len() - 0x400..]);
    h.update(device);
    h.finalize().into()
}

pub fn u32_at(d: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(d[off..off + 4].try_into().unwrap())
}

pub fn u16_at(d: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(d[off..off + 2].try_into().unwrap())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub x: u16,
    pub y: u16,
    pub start: usize,
    pub end: usize,
}

/// Directory at 0x78; a tile ends where the next one starts.
pub fn tiles(d: &[u8]) -> Vec<Tile> {
    let n = u32_at(d, 0x74) as usize;
    let dir: Vec<(u16, u16, usize)> = (0..n)
        .map(|i| {
            let o = 0x78 + 8 * i;
            (u16_at(d, o), u16_at(d, o + 2), u32_at(d, o + 4) as usize)
        })
        .collect();
    dir.iter()
        .enumerate()
        .map(|(i, &(x, y, off))| Tile {
            x,
            y,
            start: off,
            end: if i + 1 < n { dir[i + 1].2 } else { d.len() },
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
pub struct Rec {
    pub rel: usize,
    pub len: usize,
    pub pltx: usize,
}

/// The records of one tile: a gapless chain of [u32 len][u32 pltx][payload]
/// from +0x159C to the end of the tile.
pub fn records(d: &[u8], start: usize, end: usize) -> Vec<Rec> {
    let mut out = Vec::new();
    let mut rel = HEAD;
    while start + rel + 8 <= end {
        let len = u32_at(d, start + rel) as usize;
        let pltx = u32_at(d, start + rel + 4) as usize;
        if len == 0 || start + rel + 8 + len > end {
            break;
        }
        out.push(Rec { rel, len, pltx });
        rel += 8 + len;
    }
    out
}

/// PC1 then LZMA.  osm's slot area B (routing graph) is stored LZMA-only;
/// those records are recognised by the fallback, exactly as in chart.py.
pub fn decode_record(payload: &[u8], pltx: usize, rk: &[u8]) -> Result<Vec<u8>> {
    if let Ok(out) = lzma::decompress(&pc1::decrypt_payload(payload, rk), pltx) {
        return Ok(out);
    }
    match lzma::decompress(payload, pltx) {
        Ok(out) => Ok(out),
        Err(e) => bail!("record does not decode ({} plaintext bytes): {}", pltx, e),
    }
}

/// The tile's record key: 32 bytes at +0x157C, PC1 encrypted with the global key.
pub fn record_key(d: &[u8], tile_start: usize, key: &[u8]) -> Vec<u8> {
    pc1::decrypt_blob(&d[tile_start + BLOB..tile_start + HEAD], key)
}

pub struct Chart {
    pub data: Vec<u8>,
    pub key: Vec<u8>,
}

impl Chart {
    pub fn open(path: &str) -> Result<Chart> {
        let data = std::fs::read(path)?;
        if data.len() < 0x844 || u32_at(&data, 0) != MAGIC {
            bail!("{}: not a chart file (magic 0x1B62)", path);
        }
        Ok(Chart { key: global_key(&device()), data })
    }

    pub fn date(&self) -> &[u8] {
        &self.data[0x44..0x4C]
    }

    pub fn typ(&self) -> u32 {
        u32_at(&self.data, 0x4C)
    }

    pub fn layer(&self) -> u32 {
        u32_at(&self.data, 0x50)
    }

    pub fn country(&self) -> u32 {
        u32_at(&self.data, 0x54)
    }

    pub fn extra(&self) -> u32 {
        u32_at(&self.data, 0x70)
    }

    pub fn tiles(&self) -> Vec<Tile> {
        tiles(&self.data)
    }

    pub fn bound(&self, device: &[u8]) -> bool {
        header_md5(&self.data, device) == self.data[0x34..0x44]
    }

    pub fn generic(&self) -> bool {
        header_md5(&self.data, b"") == self.data[0x34..0x44]
    }

    /// Plaintext of one record of a tile.
    pub fn record(&self, tile: &Tile, rec: &Rec) -> Result<Vec<u8>> {
        let rk = record_key(&self.data, tile.start, &self.key);
        let from = tile.start + rec.rel + 8;
        decode_record(&self.data[from..from + rec.len], rec.pltx, &rk)
    }
}
