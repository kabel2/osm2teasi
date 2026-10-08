//! Command line front end.  See the subcommand list in `usage`.

use std::collections::BTreeMap;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use md5::{Digest, Md5};
use rayon::prelude::*;

use teasi::chart::{self, Chart};
use teasi::layers;

const USAGE: &str = "\
usage: teasi <command> [arguments]

  info <chart>...            header, tiles and record counts
  dump <chart> <out-dir>     write every decrypted record to a file
  md5s <chart>               'area cx cy md5' per record (to compare with Python)
  check <chart>...           parse and rebuild every record; must be byte-identical
  roundtrip <chart> [out]    decode the whole file, write it again, compare records
  index <ta chart|table>     search index: parse, rebuild and compare

The device serial comes from TEASI_DEVICE (default: the one in chart.rs).";

/// One decoded record with its place in the file.
struct Decoded {
    area: char,
    cx: usize,
    cy: usize,
    rel: usize,
    raw: Vec<u8>,
}

/// Decode every record a slot table points at, in parallel.
fn decode_all(c: &Chart) -> Result<Vec<Decoded>> {
    let tiles = c.tiles();
    let keys: Vec<Vec<u8>> = tiles
        .iter()
        .map(|t| {
            if t.start == t.end {
                Vec::new()
            } else {
                chart::record_key(&c.data, t.start, &c.key)
            }
        })
        .collect();
    let mut jobs = Vec::new();
    for (ti, t) in tiles.iter().enumerate() {
        if t.start == t.end {
            continue;
        }
        for &(area, _, _) in layers::AREAS.iter() {
            for s in layers::slots(&c.data, t, area) {
                jobs.push((ti, area, s));
            }
        }
    }
    jobs.par_iter()
        .map(|(ti, area, s)| {
            let t = &tiles[*ti];
            let from = t.start + s.rec.rel + 8;
            let raw = chart::decode_record(&c.data[from..from + s.rec.len], s.rec.pltx, &keys[*ti])
                .with_context(|| format!("{} ({},{})", area, s.cx, s.cy))?;
            Ok(Decoded { area: *area, cx: s.cx, cy: s.cy, rel: s.rec.rel, raw })
        })
        .collect()
}

fn info(path: &str) -> Result<()> {
    let c = Chart::open(path)?;
    let tiles = c.tiles();
    let empty = tiles.iter().filter(|t| t.start == t.end).count();
    println!(
        "{}\n  date {}  type {}  layer {} ({})  country {}  extra 0x{:x}",
        path,
        String::from_utf8_lossy(c.date()),
        c.typ(),
        c.layer(),
        layers::layer_name(c.layer()),
        c.country(),
        c.extra()
    );
    println!(
        "  MAC: device-bound {} | generic {}",
        c.bound(&chart::device()),
        c.generic()
    );
    let mut counts: BTreeMap<char, usize> = BTreeMap::new();
    for t in tiles.iter().filter(|t| t.start != t.end) {
        for &(area, _, _) in layers::AREAS.iter() {
            *counts.entry(area).or_default() += layers::slots(&c.data, t, area).len();
        }
    }
    let total: usize = counts.values().sum();
    let per: Vec<String> = counts
        .iter()
        .filter(|(_, &n)| n > 0)
        .map(|(a, n)| format!("{} {}", a, n))
        .collect();
    println!(
        "  {} tiles ({} empty), {} records: {}",
        tiles.len(),
        empty,
        total,
        per.join(", ")
    );
    Ok(())
}

fn dump(path: &str, outdir: &str) -> Result<()> {
    let c = Chart::open(path)?;
    std::fs::create_dir_all(outdir)?;
    let recs = decode_all(&c)?;
    let tiles = c.tiles();
    for r in &recs {
        let t = tiles
            .iter()
            .find(|t| {
                let g = layers::area_grid(r.area);
                t.x as usize == r.cx / g && t.y as usize == r.cy / g
            })
            .unwrap();
        let name = format!("{}/{}_{}_{:08x}.bin", outdir, t.x, t.y, r.rel);
        std::fs::write(name, &r.raw)?;
    }
    println!("{} records written to {}", recs.len(), outdir);
    Ok(())
}

fn md5s(path: &str) -> Result<()> {
    let c = Chart::open(path)?;
    let mut recs = decode_all(&c)?;
    recs.sort_by_key(|r| (r.area, r.cx, r.cy));
    for r in &recs {
        println!("{} {} {} {:x}", r.area, r.cx, r.cy, Md5::digest(&r.raw));
    }
    Ok(())
}

fn check(path: &str) -> Result<bool> {
    let c = Chart::open(path)?;
    let layer = c.layer();
    let recs = decode_all(&c)?;
    let results: Vec<(char, usize, usize, Result<()>)> = recs
        .par_iter()
        .map(|r| {
            let res = (|| {
                let parsed = layers::parse_record(layer, r.area, &r.raw)?;
                let again = layers::build_record(&parsed);
                if again != r.raw {
                    let at = again
                        .iter()
                        .zip(r.raw.iter())
                        .position(|(a, b)| a != b)
                        .unwrap_or_else(|| r.raw.len().min(again.len()));
                    bail!(
                        "rebuilt differs at byte {} ({} vs {} bytes)",
                        at,
                        again.len(),
                        r.raw.len()
                    );
                }
                Ok(())
            })();
            (r.area, r.cx, r.cy, res)
        })
        .collect();
    let mut per: BTreeMap<char, (usize, usize)> = BTreeMap::new();
    for (area, cx, cy, res) in &results {
        let e = per.entry(*area).or_default();
        e.0 += 1;
        if let Err(err) = res {
            e.1 += 1;
            if e.1 <= 3 {
                println!("  {} ({},{}): {}", area, cx, cy, err);
            }
        }
    }
    let bad: usize = per.values().map(|v| v.1).sum();
    let total: usize = per.values().map(|v| v.0).sum();
    let detail: Vec<String> = per
        .iter()
        .filter(|(_, v)| v.0 > 0)
        .map(|(a, v)| format!("{} {}", a, v.0))
        .collect();
    println!(
        "{}: {} records ({}), {}",
        path.rsplit('/').next().unwrap(),
        total,
        detail.join(", "),
        if bad == 0 { "all byte-identical".to_string() } else { format!("{} FAILED", bad) }
    );
    Ok(bad == 0)
}

/// Decode a file, rebuild it with the writer and compare record by record.
/// Port of tools/roundtrip.py.
fn roundtrip(path: &str, out: Option<&str>) -> Result<bool> {
    let c = Chart::open(path)?;
    let dev = chart::device();
    let t0 = std::time::Instant::now();
    let (meta, content) = teasi::writer::read_all(&c)?;
    let tails: Vec<usize> = content.iter().map(|t| t.tail.len()).collect();
    let new = teasi::writer::write_chart(&meta, &content, &dev, true, None)?;
    let secs = t0.elapsed().as_secs_f32();

    if let Some(p) = out {
        std::fs::write(p, &new)?;
    }
    let mut ok = true;
    let mut fail = |cond: bool, what: &str| {
        if !cond {
            println!("  FAILED: {}", what);
            ok = false;
        }
    };
    fail(new[..4] == c.data[..4], "magic");
    fail(new[0x44..0x58] == c.data[0x44..0x58], "date, type, layer, country");
    fail(chart::header_md5(&new, &dev) == new[0x34..0x44], "MAC");
    let old_dir: Vec<(u16, u16)> = c.tiles().iter().map(|t| (t.x, t.y)).collect();
    let nc = Chart { data: new.clone(), key: c.key.clone() };
    let new_dir: Vec<(u16, u16)> = nc.tiles().iter().map(|t| (t.x, t.y)).collect();
    fail(old_dir == new_dir, "tile directory");

    // the untouched part of the tile head (the 85 terrain cells)
    for (a, b) in c.tiles().iter().zip(nc.tiles().iter()) {
        if a.start == a.end {
            continue;
        }
        fail(
            c.data[a.start + 0x1180..a.start + chart::BLOB]
                == new[b.start + 0x1180..b.start + chart::BLOB],
            "tile head 0x1180..0x157C",
        );
    }

    let mut old_recs = decode_all(&c)?;
    let mut new_recs = decode_all(&nc)?;
    old_recs.sort_by_key(|r| (r.area, r.cx, r.cy));
    new_recs.sort_by_key(|r| (r.area, r.cx, r.cy));
    fail(old_recs.len() == new_recs.len(), "number of records");
    let mut differ = 0;
    for (o, n) in old_recs.iter().zip(new_recs.iter()) {
        if (o.area, o.cx, o.cy) != (n.area, n.cx, n.cy) || o.raw != n.raw {
            differ += 1;
        }
    }
    fail(differ == 0, &format!("{} records differ", differ));

    let (_, new_content) = teasi::writer::read_all(&nc)?;
    fail(
        new_content.iter().map(|t| t.tail.len()).collect::<Vec<_>>() == tails,
        "tails",
    );
    let old_h: Vec<String> = (0..8)
        .map(|i| format!("0x{:x}", chart::u32_at(&c.data, 0x58 + 4 * i)))
        .collect();
    let new_h: Vec<String> = (0..8)
        .map(|i| format!("0x{:x}", chart::u32_at(&new, 0x58 + 4 * i)))
        .collect();
    println!("  header 0x58..0x74 old [{}]", old_h.join(", "));
    println!("                    new [{}]", new_h.join(", "));
    println!(
        "{}: {} records identical, size {} -> {}, tails {} B, build {:.1} s",
        path.rsplit('/').next().unwrap(),
        old_recs.len(),
        c.data.len(),
        new.len(),
        tails.iter().sum::<usize>(),
        secs
    );
    Ok(ok)
}

/// Parse the ta search index and rebuild it; must come out byte-identical.
fn index(path: &str) -> Result<bool> {
    let raw = match Chart::open(path) {
        Ok(c) => teasi::ta_index::extract(&c)?,
        Err(_) => std::fs::read(path)?, // a table dumped on its own
    };
    let idx = teasi::ta_index::parse(&raw)?;
    let langs: Vec<String> = idx
        .langs
        .iter()
        .map(|(c, i)| format!("{} {}", String::from_utf8_lossy(c), i))
        .collect();
    println!(
        "{}: {} B, country {:?}, {} languages ({})",
        path.rsplit('/').next().unwrap(),
        raw.len(),
        idx.country_name(),
        idx.langs.len(),
        langs.join(", ")
    );
    let again = teasi::ta_index::build(&idx)?;
    let ok = again == raw;
    println!(
        "  {} distinct results, {} pool words, rebuild {}",
        idx.count(),
        idx.pool.len(),
        if ok { "byte-identical".to_string() } else { format!("DIFFERS ({} B)", again.len()) }
    );
    Ok(ok)
}

fn run() -> Result<bool> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        println!("{}", USAGE);
        return Ok(false);
    }
    match args[0].as_str() {
        "info" => {
            for p in &args[1..] {
                info(p)?;
            }
            Ok(true)
        }
        "dump" => {
            if args.len() != 3 {
                bail!("usage: teasi dump <chart> <out-dir>");
            }
            dump(&args[1], &args[2])?;
            Ok(true)
        }
        "md5s" => {
            md5s(&args[1])?;
            Ok(true)
        }
        "roundtrip" => {
            if args.len() < 2 {
                bail!("usage: teasi roundtrip <chart> [out]");
            }
            roundtrip(&args[1], args.get(2).map(|s| s.as_str()))
        }
        "index" => {
            let mut ok = true;
            for p in &args[1..] {
                ok &= index(p)?;
            }
            Ok(ok)
        }
        "check" => {
            let mut ok = true;
            for p in &args[1..] {
                ok &= check(p)?;
            }
            Ok(ok)
        }
        other => bail!("unknown command {:?}\n\n{}", other, USAGE),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {:#}", e);
            ExitCode::FAILURE
        }
    }
}
