//! Reading .osm.pbf files and the node index the extractors need.
//!
//! The Python side uses pyosmium (libosmium); here the blocks of the file are
//! decoded in parallel and folded into per-thread accumulators.  Coordinates
//! are converted exactly as libosmium does it -- the decimicrodegree integer
//! divided by 1e7 -- so the Teasi units come out bit-identical to the Python
//! extractors.

use anyhow::{Context, Result};
use osmpbf::{BlobDecode, BlobReader, PrimitiveBlock};
use rayon::prelude::*;

/// Teasi coordinate units: 360 degrees in 2^28 steps.
pub const SCALE: f64 = (1u64 << 28) as f64 / 360.0;

/// Blocks decoded per round; keeps memory bounded on big files.
const CHUNK: usize = 256;

/// Degrees -> Teasi units (x east of 180 W, y south of 90 N).
pub fn xy(lon: f64, lat: f64) -> (f64, f64) {
    ((lon + 180.0) * SCALE, (90.0 - lat) * SCALE)
}

/// Decimicrodegrees (what the PBF stores) -> Teasi units.
pub fn xy_dm(dm_lon: i32, dm_lat: i32) -> (f64, f64) {
    xy(f64::from(dm_lon) / 1e7, f64::from(dm_lat) / 1e7)
}

/// Fold every primitive block of `path` into an accumulator, blocks in
/// parallel.  `fold` sees whole blocks so that it can keep its own buffers.
pub fn par_blocks<T, I, F, M>(path: &str, init: I, fold: F, merge: M) -> Result<T>
where
    T: Send,
    I: Fn() -> T + Send + Sync,
    F: Fn(&mut T, &PrimitiveBlock) + Send + Sync,
    M: Fn(T, T) -> T + Send + Sync,
{
    let mut reader = BlobReader::from_path(path).with_context(|| format!("open {}", path))?;
    let mut acc = init();
    loop {
        let mut chunk = Vec::with_capacity(CHUNK);
        for blob in reader.by_ref().take(CHUNK) {
            chunk.push(blob?);
        }
        if chunk.is_empty() {
            return Ok(acc);
        }
        let parts: Vec<T> = chunk
            .par_iter()
            .map(|blob| -> Result<T> {
                let mut a = init();
                if let BlobDecode::OsmData(block) = blob.decode()? {
                    fold(&mut a, &block);
                }
                Ok(a)
            })
            .collect::<Result<Vec<T>>>()?;
        for p in parts {
            acc = merge(acc, p);
        }
    }
}

/// Node id -> coordinates, for the ids an earlier pass asked for.
///
/// Sorted ids plus a parallel array of decimicrodegrees: 16 bytes per node,
/// where the Python side keeps a libosmium index of every node in the file.
pub struct NodeIndex {
    ids: Vec<i64>,
    pos: Vec<(i32, i32)>,
}

impl NodeIndex {
    /// `ids` may contain duplicates and need not be sorted.
    pub fn new(mut ids: Vec<i64>) -> Self {
        ids.sort_unstable();
        ids.dedup();
        let pos = vec![(i32::MIN, i32::MIN); ids.len()];
        NodeIndex { ids, pos }
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn wants(&self, id: i64) -> bool {
        self.ids.binary_search(&id).is_ok()
    }

    /// Cursor for one block of nodes; see [`Probe`].
    pub fn probe(&self) -> Probe<'_> {
        Probe { ids: &self.ids, max: self.ids.last().copied().unwrap_or(i64::MIN), at: 0 }
    }

    pub fn set(&mut self, id: i64, dm_lon: i32, dm_lat: i32) {
        if let Ok(i) = self.ids.binary_search(&id) {
            self.pos[i] = (dm_lon, dm_lat);
        }
    }

    /// Teasi units, or None if the file does not contain that node.
    pub fn get(&self, id: i64) -> Option<(f64, f64)> {
        let i = self.ids.binary_search(&id).ok()?;
        let (lon, lat) = self.pos[i];
        if lon == i32::MIN && lat == i32::MIN {
            return None;
        }
        Some(xy_dm(lon, lat))
    }

    /// Decimicrodegrees as the file stores them; libosmium compares locations
    /// in these integers, so ring normalisation needs them unconverted.
    pub fn get_dm(&self, id: i64) -> Option<(i32, i32)> {
        let i = self.ids.binary_search(&id).ok()?;
        let p = self.pos[i];
        if p == (i32::MIN, i32::MIN) {
            return None;
        }
        Some(p)
    }

    pub fn missing(&self) -> usize {
        self.pos.iter().filter(|p| **p == (i32::MIN, i32::MIN)).count()
    }

    /// Fill the index from the file (one pass over all nodes).
    pub fn fill(&mut self, path: &str) -> Result<()> {
        let found = par_blocks(
            path,
            Vec::new,
            |acc: &mut Vec<(i64, i32, i32)>, block| {
                let mut probe = self.probe();
                for group in block.groups() {
                    for n in group.nodes() {
                        if probe.wants(n.id()) {
                            acc.push((n.id(), n.decimicro_lon(), n.decimicro_lat()));
                        }
                    }
                    for n in group.dense_nodes() {
                        if probe.wants(n.id()) {
                            acc.push((n.id(), n.decimicro_lon(), n.decimicro_lat()));
                        }
                    }
                }
            },
            |mut a, b| {
                a.extend(b);
                a
            },
        )?;
        for (id, lon, lat) in found {
            self.set(id, lon, lat);
        }
        Ok(())
    }
}

/// Cursor into the wanted ids of a [`NodeIndex`], for one block of nodes.
///
/// Nodes come in ascending id order in the files from Geofabrik, so a hit
/// usually sits right at the cursor or just after it, where a binary search
/// over all wanted ids costs a cache miss per step.  It saves about a second of
/// the eleven that pass 3 over Great Britain takes (21 million wanted ids, 250
/// million nodes) and, more to the point, keeps the lookup independent of how
/// big the index is.  Out-of-order ids still work, they just cost a binary
/// search.
pub struct Probe<'a> {
    ids: &'a [i64],
    max: i64,
    at: usize,
}

impl Probe<'_> {
    pub fn wants(&mut self, id: i64) -> bool {
        if id > self.max {
            return false;
        }
        match self.ids.get(self.at) {
            Some(&cur) if cur == id => return true,
            Some(&cur) if cur < id => {
                // gallop forward, then search the window we jumped over
                let mut step = 1;
                while self.ids.get(self.at + step).is_some_and(|&x| x < id) {
                    step *= 2;
                }
                let hi = (self.at + step + 1).min(self.ids.len());
                self.at += self.ids[self.at..hi].partition_point(|&x| x < id);
            }
            _ => self.at = self.ids.partition_point(|&x| x < id),
        }
        self.ids.get(self.at) == Some(&id)
    }
}

/// Compensated sum (Neumaier), which is what CPython's `sum()` does for floats
/// since 3.12.  Without this the last bit differs from the Python extractors.
pub fn fsum<I: Iterator<Item = f64>>(xs: I) -> f64 {
    let mut s = 0.0f64;
    let mut c = 0.0f64;
    for x in xs {
        let t = s + x;
        c += if s.abs() >= x.abs() { (s - t) + x } else { (x - t) + s };
        s = t;
    }
    s + c
}

/// Mean of the points, the way Python's `sum(...) / len(...)` computes it.
pub fn mean(pts: &[(f64, f64)]) -> Option<(f64, f64)> {
    if pts.is_empty() {
        return None;
    }
    let n = pts.len() as f64;
    Some((fsum(pts.iter().map(|p| p.0)) / n, fsum(pts.iter().map(|p| p.1)) / n))
}

// --------------------------------------------------------------------------
// areas the way libosmium assembles them
// --------------------------------------------------------------------------
//
// The Python extractors get their area geometry from libosmium, so anything
// derived from it -- above all the centre of an area -- only comes out the same
// if its two conventions are reproduced:
//
//   * A ring starts at its geometrically smallest vertex and repeats it at the
//     end.  (libosmium normalises every segment so that the smaller location
//     comes first, sorts the segments and starts the ring at the first of them.)
//     Which vertex is doubled shifts the mean by metres, so the rotation matters.
//   * Outer and inner rings are told apart by how deeply they are nested, not by
//     the member roles -- those are merely a hint and are routinely wrong.

/// Join ways into closed rings of node ids, each without the repeated end.
/// None when a ring stays open: libosmium then builds no area at all.
pub fn assemble_rings(ways: &[Vec<i64>]) -> Option<Vec<Vec<i64>>> {
    let mut used = vec![false; ways.len()];
    let mut out: Vec<Vec<i64>> = Vec::new();
    for start in 0..ways.len() {
        if used[start] || ways[start].len() < 2 {
            continue;
        }
        used[start] = true;
        let mut ring = ways[start].clone();
        while ring[0] != *ring.last().unwrap() {
            let end = *ring.last().unwrap();
            let mut found = false;
            for (i, w) in ways.iter().enumerate() {
                if used[i] || w.len() < 2 {
                    continue;
                }
                if w[0] == end {
                    ring.extend_from_slice(&w[1..]);
                } else if *w.last().unwrap() == end {
                    ring.extend(w[..w.len() - 1].iter().rev());
                } else {
                    continue;
                }
                used[i] = true;
                found = true;
                break;
            }
            if !found {
                return None;
            }
        }
        ring.pop();
        out.push(ring);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// The points of one ring, rotated to start at its smallest vertex and closed
/// by repeating that vertex -- see the note above.  The comparison is made on
/// the decimicrodegrees, because that is what libosmium compares.
pub fn ring_points(ring: &[i64], index: &NodeIndex) -> Option<Vec<(f64, f64)>> {
    let dm: Vec<(i32, i32)> = ring.iter().map(|id| index.get_dm(*id)).collect::<Option<_>>()?;
    let at = (0..dm.len()).min_by_key(|&i| dm[i])?;
    let mut pts: Vec<(f64, f64)> =
        dm[at..].iter().chain(dm[..at].iter()).map(|&(lon, lat)| xy_dm(lon, lat)).collect();
    pts.push(pts[0]);
    Some(pts)
}

/// Mean over the points of the outer rings; None if none of them is outer.
/// A ring nested in an even number of other rings is an outer one.
pub fn area_centre(rings: &[Vec<(f64, f64)>]) -> Option<(f64, f64)> {
    let outer: Vec<(f64, f64)> = rings
        .iter()
        .enumerate()
        .filter(|(i, r)| {
            let (x, y) = r[0];
            let depth = rings
                .iter()
                .enumerate()
                .filter(|(j, o)| j != i && crate::poly::inside(o, x, y))
                .count();
            depth % 2 == 0
        })
        .flat_map(|(_, r)| r.iter().copied())
        .collect();
    mean(&outer)
}

/// Every ring of an area, ready for [`area_centre`].  `ways` are the member
/// ways' node ids -- for a closed way that is just the way itself.
pub fn area_rings(ways: &[Vec<i64>], index: &NodeIndex) -> Option<Vec<Vec<(f64, f64)>>> {
    assemble_rings(ways)?.iter().map(|r| ring_points(r, index)).collect()
}
