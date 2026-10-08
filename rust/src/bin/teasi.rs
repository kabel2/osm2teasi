//! Command line front end.  See the subcommand list in `usage`.

use std::io::Write;
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
  md5s <chart>               'area cx cy md5' per record (to compare two charts)
  check <chart>...           parse and rebuild every record; must be byte-identical
  roundtrip <chart> [out]    decode the whole file, write it again, compare records
  index <ta chart|table>     search index: parse, rebuild and compare
  addr <file.osm.pbf> [out]  addresses, places and interpolation ways from OSM
                             (out: one sorted line per address, places and ways)
  poi <file.osm.pbf> <out>   POI candidates as a canonical dump
  osmpoi <file.osm.pbf> <area.poly> <out chart> [YYYYMMDD] [--country=N]
  osmpoint <file.osm.pbf> <area.poly> <out chart> [YYYYMMDD] [--country=N]
                             compile that layer (country 4 = Denmark,
                             17 = United Kingdom)
  area <file.osm.pbf> <out>  areas and coastline as a canonical dump
  land <land_polygons.shp> <area.poly>
                             count and area of the worldwide land polygons
  osmarea <file.osm.pbf> <area.poly> <original|-> <out chart> [YYYYMMDD]
                             compile the area layer; the original supplies the
                             sea outside the boundary, without one pass
                             --land=<land_polygons.shp> (needs libgeos, see
                             src/geos.rs)
  ways <file.osm.pbf> <out>  the road and line ways as a canonical dump
  osm <file.osm.pbf> <area.poly> <original|-> <out chart> [YYYYMMDD]
                             compile the street layer; --heights=<file> adds the
                             ascents (from `teasi dem`), --name=<country>
                             the country name of the A records (needs libgeos)
  ta <file.osm.pbf> <area.poly> <out chart> [YYYYMMDD]
                             compile the address search incl. its search index
                             (--country=N, --name=<country>; needs libgeos)
  all <file.osm.pbf> <area.poly> <out dir> [YYYYMMDD]
                             the elevation grid and all six layer files of one
                             country in one run, named like the originals:
                             --country=<name|code> picks the country (a name
                             from the firmware's list, see chart.rs), --only=
                             a subset of the layers, --land= the sea and the
                             map images, --tiles= where the DEM tiles are
                             cached (default <out dir>/dem_tiles),
                             --original=<chart> an original file of that
                             country, which supplies the sea outside the
                             boundary and any tiles the extract does not cover
  dem <area.poly> <tile dir> <out.bin>
                             download the Copernicus DEM GLO-90 for that area
                             and write the elevation grid --heights= reads
                             (--sigma=S smooths it, default 1)
  terrain <heights> <area.poly> <out chart> [YYYYMMDD]
                             compile the elevation model (heights from
                             `teasi dem`); --land=<land_polygons.shp>
                             --area=<file.osm.pbf> add the map images (needs
                             libgeos), --rate=R the compression ratio,
                             --only=x,y builds a single region

Every compiler takes --generic: the file is then signed for no device in
particular and the firmware binds it to the first one that opens it, so the
same file works on any device whose serial starts with the same eight digits.
Without it the file is signed for TEASI_DEVICE (default: the serial in
chart.rs), like every original file.";

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
    println!(
        "  packages.xml: size {} md5 {:x}",
        c.data.len(),
        chart::package_md5(&c.data)
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
fn roundtrip(path: &str, out: Option<&str>) -> Result<bool> {
    let c = Chart::open(path)?;
    let sign = chart::Signer::bound();
    let t0 = std::time::Instant::now();
    let (meta, content) = teasi::writer::read_all(&c)?;
    let tails: Vec<usize> = content.iter().map(|t| t.tail.len()).collect();
    let new = teasi::writer::write_chart(&meta, &content, &sign, None)?;
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
    fail(sign.mac(&new) == new[0x34..0x44], "MAC");
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

/// Clean a tag value for the canonical dump (both sides do the same).
fn flat(s: Option<&String>) -> String {
    s.map(|v| v.replace(['\t', '\n', '\r'], " ")).unwrap_or_default()
}

fn bits(v: f64) -> String {
    format!("{:016x}", v.to_bits())
}

/// Extract addresses from a .osm.pbf.  With `out`, write a canonical dump: one
/// sorted line per address, place and interpolation way, stable enough to diff
/// two extracts against each other.
fn addr(path: &str, out: Option<&str>) -> Result<bool> {
    let t0 = std::time::Instant::now();
    let ex = teasi::addr::extract(path, &|s| println!("  {}", s))?;
    println!(
        "{}: {} addresses, {} places, {} interpolations in {:.1} s",
        path.rsplit('/').next().unwrap(),
        ex.addr.len(),
        ex.places.len(),
        ex.interp.len(),
        t0.elapsed().as_secs_f32()
    );
    if let Some(dst) = out {
        let mut lines: Vec<String> = Vec::with_capacity(ex.addr.len() + ex.places.len());
        for a in &ex.addr {
            lines.push(format!(
                "A\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                bits(a.x),
                bits(a.y),
                flat(a.tags.hn.as_ref()),
                flat(a.tags.street.as_ref()),
                flat(a.tags.postcode.as_ref()),
                flat(a.tags.city.as_ref()),
                flat(a.tags.place.as_ref()),
                flat(a.tags.suburb.as_ref())
            ));
        }
        for p in &ex.places {
            lines.push(format!(
                "P\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                bits(p.x),
                bits(p.y),
                flat(p.tags.place.as_ref()),
                flat(p.tags.name.as_ref()),
                flat(p.tags.name_en.as_ref()),
                flat(p.tags.population.as_ref()),
                flat(p.tags.is_in.as_ref()),
                if p.area { "area" } else { "" }
            ));
        }
        for i in &ex.interp {
            for (n, (x, y, hn, st)) in i.pts.iter().enumerate() {
                lines.push(format!(
                    "I\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    bits(*x),
                    bits(*y),
                    i.kind.replace(['\t', '\n', '\r'], " "),
                    flat(i.street.as_ref()),
                    i.pts.len(),
                    n,
                    flat(hn.as_ref()),
                    flat(st.as_ref())
                ));
            }
        }
        lines.sort();
        std::fs::write(dst, lines.join("\n") + "\n")?;
        println!("  {} lines -> {}", lines.len(), dst);
    }
    Ok(true)
}

/// POI candidates as a canonical dump: one sorted line each, stable enough to
/// diff two extracts against each other.
fn poi(path: &str, dst: &str) -> Result<bool> {
    let t0 = std::time::Instant::now();
    let cands = teasi::poi::extract(path, &|s| println!("  {}", s))?;
    let mut lines: Vec<String> = cands
        .iter()
        .filter_map(|c| {
            let f = c.poi.as_ref()?;
            let centre = match c.centre {
                Some((x, y)) => format!("{}:{}", bits(x), bits(y)),
                None => "-".to_string(),
            };
            Some(format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                c.kind,
                c.id,
                f.typ,
                bits(c.x),
                bits(c.y),
                centre,
                flat(Some(&f.name)),
                flat(Some(&f.attrs))
            ))
        })
        .collect();
    lines.sort();
    std::fs::write(dst, lines.join("\n") + "\n")?;
    println!(
        "{}: {} candidates -> {} in {:.1} s",
        path.rsplit('/').next().unwrap(),
        lines.len(),
        dst,
        t0.elapsed().as_secs_f32()
    );
    Ok(true)
}

/// Compile the osmpoi or osmpoint layer from a .osm.pbf.  Both layers read
/// the same candidates, so one pass over the file serves either.
fn compile_poi(layer: &str, args: &[String], country: u32, sign: &chart::Signer) -> Result<bool> {
    if args.len() < 3 {
        bail!("usage: teasi {} <file.osm.pbf> <area.poly> <out chart> [YYYYMMDD]", layer);
    }
    let (src, area, dst) = (&args[0], &args[1], &args[2]);
    let date = match args.get(3) {
        Some(d) => d.clone(),
        None => chart::today(),
    };
    let t0 = std::time::Instant::now();
    let cands = teasi::poi::extract(src, &|s| println!("  {}", s))?;
    let n = cands.len();
    let rings = teasi::poly::load(area)?;
    let (what, kept, d) = if layer == "osmpoint" {
        let pts = teasi::osmpoint::collect(&cands, Some(&rings));
        let d = teasi::osmpoint::build(&pts, date.as_bytes(), country, sign)?;
        ("seamarks", pts.len(), d)
    } else {
        let pois = teasi::osmpoi::collect(&cands, Some(&rings));
        let d = teasi::osmpoi::build(&pois, date.as_bytes(), country, sign)?;
        ("POIs", pois.len(), d)
    };
    std::fs::write(dst, &d)?;
    println!(
        "{} candidates, {} {}, {} B -> {} in {:.1} s",
        n,
        kept,
        what,
        d.len(),
        dst,
        t0.elapsed().as_secs_f32()
    );
    Ok(true)
}

/// Canonical dump of the areas and the coastline: one line each, with the ring
/// coordinates hashed, stable enough to diff two extracts against each other.
fn area(path: &str, dst: &str) -> Result<()> {
    let ex = teasi::area::extract(path, &|s| println!("  {}", s))?;
    let mut out = std::io::BufWriter::new(std::fs::File::create(dst)?);
    let digest = |r: &[(i32, i32)]| {
        let mut h = Md5::new();
        for &(x, y) in r {
            h.update(x.to_le_bytes());
            h.update(y.to_le_bytes());
        }
        format!("{:x}", h.finalize())[..8].to_string()
    };
    let mut areas: Vec<&teasi::area::Area> = ex.areas.iter().collect();
    areas.sort_by_key(|a| a.id);
    for a in &areas {
        write!(out, "a\t{}\t{}", a.id, a.polys.len())?;
        for p in &a.polys {
            write!(
                out,
                "\t{},{},{},{},{}",
                p.outer.len(),
                p.inners.len(),
                p.outer[0].0,
                p.outer[0].1,
                digest(&p.outer)
            )?;
            for i in &p.inners {
                write!(out, ",{}:{}", i.len(), digest(i))?;
            }
        }
        writeln!(out)?;
    }
    let mut coast: Vec<&teasi::area::Coast> = ex.coast.iter().collect();
    coast.sort_by_key(|c| c.id);
    for c in &coast {
        writeln!(
            out,
            "c\t{}\t{}\t{}\t{}\t{}\t{}",
            c.id,
            c.first,
            c.last,
            c.islet as u8,
            c.ring.len(),
            digest(&c.ring)
        )?;
    }
    println!("{} areas, {} coastline ways -> {}", areas.len(), coast.len(), dst);
    Ok(())
}

/// Count and total area of the land polygons around a boundary.
fn land(shp: &str, area: &str) -> Result<()> {
    teasi::geos::available()?;
    let l = teasi::land::extract(shp, &teasi::poly::load(area)?)?;
    let total: f64 = l.iter().map(|p| p.area()).sum();
    println!("{} land polygons, total area {}", l.len(), total);
    if let Some(f) = l.first() {
        let b = f.bounds();
        println!("first: {:?} {} {}", b, f.area(), f.exterior().len());
    }
    Ok(())
}

/// Canonical dump of the way extractor: one line per way, nodes hashed.
fn ways(path: &str, dst: &str) -> Result<()> {
    let w = teasi::way::extract(path, &|s| println!("  {}", s))?;
    let mut out = std::io::BufWriter::new(std::fs::File::create(dst)?);
    for k in 0..w.ids.len() {
        let t = &w.tags[k];
        let rows = w.nodes(k);
        let mut h = Md5::new();
        for i in rows.clone() {
            h.update(w.nid[i].to_le_bytes());
            h.update(w.x[i].to_bits().to_le_bytes());
            h.update(w.y[i].to_bits().to_le_bytes());
        }
        let md5 = format!("{:x}", h.finalize())[..8].to_string();
        let rels = w.rels.get(&w.ids[k]).map_or(&[][..], |v| &v[..]);
        let (kind, ty, name) = match teasi::osm::road_class(t) {
            Some(c) => ("r", c, t.get("name").unwrap_or("").to_string()),
            None => match teasi::osm::line_type(t) {
                Some(teasi::osm::LineType::A3(ty)) => ("3", ty, String::new()),
                Some(teasi::osm::LineType::A4(ty, nm)) => ("4", ty, nm),
                None => ("-", 0, String::new()),
            },
        };
        writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            w.ids[k],
            kind,
            ty,
            teasi::osm::flags(t, rels),
            rows.len(),
            md5,
            name
        )?;
    }
    println!("{} ways -> {}", w.ids.len(), dst);
    Ok(())
}

fn compile_osm(
    args: &[String],
    country: u32,
    name: &str,
    heights: Option<&str>,
    sign: &chart::Signer,
) -> Result<bool> {
    if args.len() < 4 {
        bail!("usage: teasi osm <file.osm.pbf> <area.poly> <original|-> <out chart> [YYYYMMDD]");
    }
    teasi::geos::available()?;
    let (src, area, orig, dst) = (&args[0], &args[1], &args[2], &args[3]);
    let date = match args.get(4) {
        Some(d) => d.clone(),
        None => chart::today(),
    };
    let t0 = std::time::Instant::now();
    println!("  libgeos {}", teasi::geos::version()?);
    let hts = match heights {
        Some(p) => {
            let h = teasi::heights::Heights::load(p)?;
            println!("  {} from {}", h.describe(), p);
            Some(h)
        }
        None => None,
    };
    let mut w = teasi::way::extract(src, &|s| println!("  {}", s))?;
    let original = if orig == "-" { None } else { Some(Chart::open(orig)?) };
    let rings = teasi::poly::load(area)?;
    let p = teasi::osmarea::boundary_at(&rings, 1.0)?;
    let d = teasi::osm::build(
        &mut w,
        &p,
        original.as_ref(),
        date.as_bytes(),
        hts.as_ref(),
        country,
        name,
        sign,
        &|s| println!("  {}", s),
    )?;
    std::fs::write(dst, &d)?;
    println!("{} B -> {} in {:.1} s", d.len(), dst, t0.elapsed().as_secs_f32());
    Ok(true)
}

fn compile_ta(args: &[String], country: u32, name: &str, sign: &chart::Signer) -> Result<bool> {
    if args.len() < 3 {
        bail!("usage: teasi ta <file.osm.pbf> <area.poly> <out chart> [YYYYMMDD]");
    }
    teasi::geos::available()?;
    let (src, area, dst) = (&args[0], &args[1], &args[2]);
    let date = match args.get(3) {
        Some(d) => d.clone(),
        None => chart::today(),
    };
    let t0 = std::time::Instant::now();
    println!("  libgeos {}", teasi::geos::version()?);
    let log = |s: &str| println!("  {}", s);
    let mut ad = teasi::addr::extract(src, &log)?;
    let mut w = teasi::way::extract(src, &log)?;
    let rings = teasi::poly::load(area)?;
    let p = teasi::osmarea::boundary_at(&rings, 1.0)?;
    let d = teasi::ta::build(
        &mut w,
        &mut ad,
        &p,
        date.as_bytes(),
        country,
        name,
        sign,
        &log,
    )?;
    std::fs::write(dst, &d)?;
    println!("{} B -> {} in {:.1} s", d.len(), dst, t0.elapsed().as_secs_f32());
    Ok(true)
}

fn compile_osmarea(
    args: &[String],
    country: u32,
    land: Option<&str>,
    sign: &chart::Signer,
) -> Result<bool> {
    if args.len() < 4 {
        bail!("usage: teasi osmarea <file.osm.pbf> <area.poly> <original|-> <out chart> [YYYYMMDD]");
    }
    teasi::geos::available()?;
    let (src, area, orig, dst) = (&args[0], &args[1], &args[2], &args[3]);
    let date = match args.get(4) {
        Some(d) => d.clone(),
        None => chart::today(),
    };
    let t0 = std::time::Instant::now();
    println!("  libgeos {}", teasi::geos::version()?);
    let ex = teasi::area::extract(src, &|s| println!("  {}", s))?;

    let original = if orig == "-" { None } else { Some(Chart::open(orig)?) };
    let rings = teasi::poly::load(area)?;
    let land = match land {
        Some(shp) => {
            let l = teasi::land::extract(shp, &rings)?;
            println!("  {} land polygons from {}", l.len(), shp);
            Some(l)
        }
        None => None,
    };
    let p = teasi::osmarea::boundary(&rings)?;
    let d = teasi::osmarea::build(
        &ex,
        &p,
        original.as_ref(),
        land,
        date.as_bytes(),
        country,
        sign,
        &|s| println!("  {}", s),
    )?;
    std::fs::write(dst, &d)?;
    println!("{} B -> {} in {:.1} s", d.len(), dst, t0.elapsed().as_secs_f32());
    Ok(true)
}

/// Download the Copernicus DEM for an area and write the flat heights file.
fn dem(args: &[String], sigma: f64) -> Result<bool> {
    if args.len() < 3 {
        bail!("usage: teasi dem <area.poly> <tile dir> <out.bin> [--sigma=S]");
    }
    let (poly, tdir, dst) = (&args[0], &args[1], &args[2]);
    let t0 = std::time::Instant::now();
    let rings = teasi::poly::load(poly)?;
    let g = teasi::dem::build(&rings, std::path::Path::new(tdir), sigma, &|s| {
        println!("  {}", s)
    })?;
    g.write(dst)?;
    println!(
        "grid {}x{}, {:.6} deg, sigma {} -> {} in {:.1} s",
        g.rows,
        g.cols,
        g.step,
        sigma,
        dst,
        t0.elapsed().as_secs_f32()
    );
    Ok(true)
}

fn compile_terrain(
    args: &[String],
    country: u32,
    rate: f32,
    land: Option<&str>,
    area: Option<&str>,
    only: Option<(i64, i64)>,
    sign: &chart::Signer,
) -> Result<bool> {
    if args.len() < 3 {
        bail!(
            "usage: teasi terrain <heights> <area.poly> <out chart> [YYYYMMDD] \
             [--rate=R] [--land=<land_polygons.shp> --area=<file.osm.pbf>] [--only=x,y]"
        );
    }
    let (src, poly, dst) = (&args[0], &args[1], &args[2]);
    let date = match args.get(3) {
        Some(d) => d.clone(),
        None => chart::today(),
    };
    let t0 = std::time::Instant::now();
    let g = teasi::heights::Heights::load(src)?;
    println!("  {} from {}", g.describe(), src);
    println!("  openjpeg {}", teasi::raster::openjpeg_version());
    let rings = teasi::poly::load(poly)?;
    let land = match land {
        Some(shp) => {
            teasi::geos::available()?;
            let l = teasi::land::extract(shp, &rings)?;
            println!("  {} land polygons from {}", l.len(), shp);
            Some(l)
        }
        None => None,
    };
    let areas = match area {
        Some(pbf) => Some(teasi::area::extract(pbf, &|s| println!("  {}", s))?),
        None => None,
    };
    let d = teasi::terrain::build(
        &g,
        &rings,
        date.as_bytes(),
        country,
        rate,
        sign,
        land.as_deref(),
        areas.as_ref(),
        only,
        &|s| println!("  {}", s),
    )?;
    std::fs::write(dst, &d)?;
    println!("{} B -> {} in {:.1} s", d.len(), dst, t0.elapsed().as_secs_f32());
    Ok(true)
}

/// The six layers of a map, in the order `all` builds them.
const LAYERS: [&str; 6] = ["osmpoi", "osmpoint", "osmarea", "osm", "ta", "terrain"];

/// Build a whole country: the elevation grid and all six layer files, named
/// like the originals, into one directory.
#[allow(clippy::too_many_arguments)]
fn compile_all(
    args: &[String],
    cname: &str,
    name: Option<&str>,
    land: Option<&str>,
    tiles: Option<&str>,
    only: Option<&str>,
    prefix: Option<&str>,
    orig: Option<&str>,
    rate: f32,
    sigma: f64,
    sign: &chart::Signer,
) -> Result<bool> {
    if args.len() < 3 {
        bail!(
            "usage: teasi all <file.osm.pbf> <area.poly> <out dir> [YYYYMMDD] \
             --country=<name|code> [--name=<country>] [--prefix=<FileName>] \
             [--land=<land_polygons.shp>] [--tiles=<dem dir>] [--only=osm,ta] \
             [--original=<chart>]"
        );
    }
    let (src, area, dir) = (&args[0], &args[1], &args[2]);
    let date = match args.get(3) {
        Some(d) => d.clone(),
        None => chart::today(),
    };
    let (country, known, file) = chart::country(cname)?;
    let name = match name.or(known) {
        Some(n) => n.to_string(),
        None => bail!("--country={} is not in the list, so --name=<country> is needed", country),
    };
    let prefix = match prefix.or(file) {
        Some(p) => p.to_string(),
        None => name.replace(' ', ""),
    };
    let want: Vec<&str> = match only {
        Some(l) => {
            let sel: Vec<&str> = l.split(',').map(str::trim).collect();
            for s in &sel {
                if !LAYERS.contains(s) {
                    bail!("--only: unknown layer {:?}; one of {}", s, LAYERS.join(", "));
                }
            }
            LAYERS.iter().filter(|l| sel.contains(*l)).copied().collect()
        }
        None => LAYERS.to_vec(),
    };
    // fail before the first half hour of work, not after it
    if want.iter().any(|l| ["osmarea", "osm", "ta"].contains(l))
        || (want.contains(&"terrain") && land.is_some())
    {
        teasi::geos::available()?;
    }
    if want.contains(&"osmarea") && land.is_none() && orig.is_none() {
        println!("  note: without --land= or --original= the areas get no sea");
    }
    std::fs::create_dir_all(dir)?;
    let tdir = match tiles {
        Some(t) => t.to_string(),
        None => format!("{}/dem_tiles", dir),
    };
    let grid = format!("{}/dem.bin", dir);
    let out = |layer: &str| format!("{}/{}_{}.v{}", dir, prefix, layer, date);

    println!(
        "{} (country {}), {} -> {}, {}",
        name,
        country,
        date,
        dir,
        if sign.bind { "signed for this device" } else { "signed generically" }
    );
    // only the ascents of the street layer and the terrain layer need heights
    let heights = want.iter().any(|l| ["osm", "terrain"].contains(l));
    let t0 = std::time::Instant::now();
    let steps = want.len() + usize::from(heights);
    let step = |n: usize, what: &str| println!("\n[{}/{}] {}", n, steps, what);

    if heights {
        step(1, "elevation grid");
        dem(&[area.clone(), tdir, grid.clone()], sigma)?;
    }

    for (i, layer) in want.iter().enumerate() {
        step(i + 1 + usize::from(heights), layer);
        let dst = out(layer);
        let mut a: Vec<String> = vec![src.clone(), area.clone()];
        match *layer {
            "osmpoi" | "osmpoint" => {
                a.extend([dst, date.clone()]);
                compile_poi(layer, &a, country, sign)?;
            }
            "osmarea" => {
                a.extend([orig.unwrap_or("-").to_string(), dst, date.clone()]);
                compile_osmarea(&a, country, land, sign)?;
            }
            "osm" => {
                a.extend([orig.unwrap_or("-").to_string(), dst, date.clone()]);
                compile_osm(&a, country, &name, Some(&grid), sign)?;
            }
            "ta" => {
                a.extend([dst, date.clone()]);
                compile_ta(&a, country, &name, sign)?;
            }
            "terrain" => {
                let a = vec![grid.clone(), area.clone(), dst, date.clone()];
                compile_terrain(&a, country, rate, land, Some(src), None, sign)?;
            }
            _ => unreachable!(),
        }
    }

    println!("\n{} in {:.0} s:", name, t0.elapsed().as_secs_f32());
    for layer in &want {
        let d = std::fs::read(out(layer))?;
        println!(
            "  {}_{}.v{}  size {} md5 {:x}",
            prefix,
            layer,
            date,
            d.len(),
            chart::package_md5(&d)
        );
    }
    println!(
        "\nCopy these into BikeNav/Map/Countries/ on the device.  They have no entry in\n\
         packages.xml, so they are loaded without a checksum check and nothing there has to\n\
         be touched (see docs/CHART_FILES.md 5.4)."
    );
    Ok(true)
}

fn run() -> Result<bool> {
    let all: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<String> = all.iter().filter(|a| !a.starts_with("--")).cloned().collect();
    let opts: Vec<(&str, &str)> = all
        .iter()
        .filter(|a| a.starts_with("--"))
        .map(|a| a[2..].split_once('=').unwrap_or((&a[2..], "")))
        .collect();
    let opt = |name: &str| opts.iter().find(|(k, _)| *k == name).map(|(_, v)| *v);
    // --generic: no serial in the MAC, so any device can bind the file itself
    let sign = match opt("generic") {
        Some(_) => chart::Signer::generic(),
        None => chart::Signer::bound(),
    };
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
        "addr" => {
            if args.len() < 2 {
                bail!("usage: teasi addr <file.osm.pbf> [dump]");
            }
            addr(&args[1], args.get(2).map(|s| s.as_str()))
        }
        "poi" => {
            if args.len() < 3 {
                bail!("usage: teasi poi <file.osm.pbf> <out>");
            }
            poi(&args[1], &args[2])
        }
        "area" => {
            if args.len() != 3 {
                bail!("usage: teasi area <file.osm.pbf> <out>");
            }
            area(&args[1], &args[2])?;
            Ok(true)
        }
        "land" => {
            if args.len() != 3 {
                bail!("usage: teasi land <land_polygons.shp> <area.poly>");
            }
            land(&args[1], &args[2])?;
            Ok(true)
        }
        "ways" => {
            if args.len() != 3 {
                bail!("usage: teasi ways <file.osm.pbf> <out>");
            }
            ways(&args[1], &args[2])?;
            Ok(true)
        }
        "osm" => {
            let country = match opt("country") {
                Some(v) => v.parse().context("--country")?,
                None => 4,
            };
            compile_osm(&args[1..], country, opt("name").unwrap_or("Denmark"), opt("heights"), &sign)
        }
        "ta" => {
            let country = match opt("country") {
                Some(v) => v.parse().context("--country")?,
                None => 17,
            };
            compile_ta(&args[1..], country, opt("name").unwrap_or("United Kingdom"), &sign)
        }
        "osmarea" => {
            let country = match opt("country") {
                Some(v) => v.parse().context("--country")?,
                None => 4,
            };
            compile_osmarea(&args[1..], country, opt("land"), &sign)
        }
        "osmpoi" | "osmpoint" => {
            let country = match opt("country") {
                Some(v) => v.parse().context("--country")?,
                None => 4,
            };
            compile_poi(&args[0], &args[1..], country, &sign)
        }
        "all" => {
            let rate = match opt("rate") {
                Some(v) => v.parse().context("--rate")?,
                None => teasi::terrain::RATE,
            };
            let sigma = match opt("sigma") {
                Some(v) => v.parse().context("--sigma")?,
                None => teasi::dem::SIGMA,
            };
            compile_all(
                &args[1..],
                opt("country").unwrap_or("Denmark"),
                opt("name"),
                opt("land"),
                opt("tiles"),
                opt("only"),
                opt("prefix"),
                opt("original"),
                rate,
                sigma,
                &sign,
            )
        }
        "dem" => {
            let sigma = match opt("sigma") {
                Some(v) => v.parse().context("--sigma")?,
                None => teasi::dem::SIGMA,
            };
            dem(&args[1..], sigma)
        }
        "terrain" => {
            let country = match opt("country") {
                Some(v) => v.parse().context("--country")?,
                None => 17,
            };
            let rate = match opt("rate") {
                Some(v) => v.parse().context("--rate")?,
                None => teasi::terrain::RATE,
            };
            let only = match opt("only") {
                Some(v) => match v.split_once(',') {
                    Some((x, y)) => Some((x.trim().parse()?, y.trim().parse()?)),
                    None => bail!("--only=x,y"),
                },
                None => None,
            };
            compile_terrain(&args[1..], country, rate, opt("land"), opt("area"), only, &sign)
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
