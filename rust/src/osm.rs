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

/// All tags of one object, in file order.  Only needed where the set of keys is
/// open-ended, as in `seamark:light:3:colour`; everything else reads a fixed
/// set of keys and does not pay for this.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagMap(Vec<(Box<str>, Box<str>)>);

impl TagMap {
    pub fn of<'a, I: Iterator<Item = (&'a str, &'a str)>>(it: I) -> Self {
        TagMap(it.map(|(k, v)| (k.into(), v.into())).collect())
    }

    /// The value of `key`; the last one wins, as in a Python dict.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.iter().rev().find(|(k, _)| &**k == key).map(|(_, v)| &**v)
    }

    /// Like `t.get(k)` where an empty value counts as absent.
    pub fn non_empty(&self, key: &str) -> Option<&str> {
        self.get(key).filter(|v| !v.is_empty())
    }

    /// First of `keys` with a non-empty value.
    pub fn first(&self, keys: &[&str]) -> Option<&str> {
        keys.iter().find_map(|k| self.non_empty(k))
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (&**k, &**v))
    }
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

/// Rings of locations from the segments of several ways, the way libosmium
/// assembles an area: every segment is normalised so that the smaller location
/// comes first, pairs of identical segments cancel each other out -- that is how
/// two ways running along each other merge their rings, and how a spike that
/// doubles back disappears -- and the rings are then walked out of what is left.
/// None when a ring stays open: libosmium then builds no area at all.
pub fn assemble_segments(ways: &[Vec<(i32, i32)>]) -> Option<Vec<Vec<(i32, i32)>>> {
    let mut segs: Vec<((i32, i32), (i32, i32))> = Vec::new();
    for w in ways {
        for p in w.windows(2) {
            if p[0] != p[1] {
                segs.push(if p[0] < p[1] { (p[0], p[1]) } else { (p[1], p[0]) });
            }
        }
    }
    segs.sort_unstable();
    let mut kept: Vec<((i32, i32), (i32, i32))> = Vec::new();
    let mut i = 0;
    while i < segs.len() {
        let mut j = i;
        while j < segs.len() && segs[j] == segs[i] {
            j += 1;
        }
        if (j - i) % 2 == 1 {
            kept.push(segs[i]);
        }
        i = j;
    }
    let mut adj: std::collections::HashMap<(i32, i32), Vec<usize>> =
        std::collections::HashMap::new();
    for (k, s) in kept.iter().enumerate() {
        adj.entry(s.0).or_default().push(k);
        adj.entry(s.1).or_default().push(k);
    }
    let mut used = vec![false; kept.len()];
    let mut out: Vec<Vec<(i32, i32)>> = Vec::new();
    for start in 0..kept.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let first = kept[start].0;
        let mut ring = vec![first];
        let mut end = kept[start].1;
        while end != first {
            ring.push(end);
            let next = adj.get(&end)?.iter().copied().find(|&k| !used[k])?;
            used[next] = true;
            end = if kept[next].0 == end { kept[next].1 } else { kept[next].0 };
        }
        out.push(ring);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Split a closed sequence of node locations wherever it visits a location
/// twice, the way libosmium's assembler does: a ring touching itself falls
/// apart into two rings (an outer one and a hole, as the nesting then shows).
/// Pieces with fewer than three points are no rings and are dropped.
pub fn split_rings(locs: &[(i32, i32)]) -> Vec<Vec<(i32, i32)>> {
    let mut out: Vec<Vec<(i32, i32)>> = Vec::new();
    let mut stack: Vec<(i32, i32)> = Vec::new();
    let mut at: std::collections::HashMap<(i32, i32), usize> = std::collections::HashMap::new();
    for &p in locs {
        match at.get(&p) {
            Some(&i) => {
                // the piece back to the earlier visit closes a ring of its own;
                // the shared location stays for the rest of the ring
                let piece = stack.split_off(i + 1);
                for q in &piece {
                    at.remove(q);
                }
                let mut ring = vec![p];
                ring.extend(piece);
                if ring.len() > 2 {
                    out.push(ring);
                }
            }
            None => {
                at.insert(p, stack.len());
                stack.push(p);
            }
        }
    }
    if stack.len() > 2 {
        out.push(stack);
    }
    out
}

/// Every ring of an area, assembled, split and rotated so that it starts at its
/// smallest location -- the comparison is on the decimicrodegree integers, as in
/// libosmium.  `ways` are the member ways' node ids; for a closed way that is
/// just the way itself.  None when a ring stays open or a node is missing.
pub fn area_loc_rings(ways: &[Vec<i64>], index: &NodeIndex) -> Option<Vec<Vec<(i32, i32)>>> {
    let locs: Vec<Vec<(i32, i32)>> = ways
        .iter()
        .map(|w| w.iter().map(|id| index.get_dm(*id)).collect::<Option<Vec<_>>>())
        .collect::<Option<_>>()?;
    let rings = assemble_segments(&locs)?;
    rings
        .iter()
        .flat_map(|r| split_rings(r))
        .map(|r| {
            let at = (0..r.len()).min_by_key(|&i| r[i])?;
            Some(r[at..].iter().chain(r[..at].iter()).copied().collect())
        })
        .collect()
}

/// The same rings in Teasi units, each closed by repeating its first point --
/// so the mean over them double-counts that point, exactly as in Python.
pub fn area_rings(ways: &[Vec<i64>], index: &NodeIndex) -> Option<Vec<Vec<(f64, f64)>>> {
    Some(
        area_loc_rings(ways, index)?
            .iter()
            .map(|r| {
                let mut pts: Vec<(f64, f64)> =
                    r.iter().map(|&(lon, lat)| xy_dm(lon, lat)).collect();
                pts.push(pts[0]);
                pts
            })
            .collect(),
    )
}

/// Mean over the points of the outer rings; None if none of them is outer.
/// A ring nested in an even number of other rings is an outer one.
pub fn area_centre(rings: &[Vec<(f64, f64)>]) -> Option<(f64, f64)> {
    // two rings touching in a vertex make the test point ambiguous, so take the
    // first point of the ring that is not a corner of the ring tested against
    let is_in = |r: &[(f64, f64)], o: &[(f64, f64)]| -> bool {
        let (x, y) = *r.iter().find(|p| !o.contains(p)).unwrap_or(&r[0]);
        crate::poly::inside(o, x, y)
    };
    let outer: Vec<(f64, f64)> = rings
        .iter()
        .enumerate()
        .filter(|(i, r)| {
            let depth =
                rings.iter().enumerate().filter(|(j, o)| j != i && is_in(r, o)).count();
            depth % 2 == 0
        })
        .flat_map(|(_, r)| r.iter().copied())
        .collect();
    mean(&outer)
}

