//! A uniform bucket grid over points, for the neighbour searches of the ta
//! layer.  scipy uses a kd-tree (`cKDTree`); with a handful of neighbours out
//! of a dense cloud the result is the same set, and equal distances are broken
//! by index here.

/// Points in buckets of about two points each.
pub struct Grid {
    pts: Vec<(f64, f64)>,
    x0: f64,
    y0: f64,
    size: f64,
    nx: i64,
    ny: i64,
    start: Vec<u32>,
    items: Vec<u32>,
}

impl Grid {
    pub fn new(pts: &[(f64, f64)]) -> Grid {
        let n = pts.len();
        let (mut x0, mut x1) = (f64::MAX, f64::MIN);
        let (mut y0, mut y1) = (f64::MAX, f64::MIN);
        for &(x, y) in pts {
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
        }
        if n == 0 {
            x0 = 0.0;
            y0 = 0.0;
            x1 = 0.0;
            y1 = 0.0;
        }
        // about as many buckets as points, whatever the aspect ratio: a
        // density-based size collapses to zero when the points are collinear
        let span = (x1 - x0).max(y1 - y0).max(1e-9);
        let size = (span / (n as f64).sqrt().ceil().max(1.0)).max(1e-9);
        let nx = (((x1 - x0) / size) as i64 + 1).max(1);
        let ny = (((y1 - y0) / size) as i64 + 1).max(1);
        let cell = |p: (f64, f64)| {
            let cx = (((p.0 - x0) / size) as i64).clamp(0, nx - 1);
            let cy = (((p.1 - y0) / size) as i64).clamp(0, ny - 1);
            (cy * nx + cx) as usize
        };
        let mut count = vec![0u32; (nx * ny) as usize + 1];
        for &p in pts {
            count[cell(p) + 1] += 1;
        }
        for i in 0..(nx * ny) as usize {
            count[i + 1] += count[i];
        }
        let start = count.clone();
        let mut items = vec![0u32; n];
        let mut at = count;
        for (i, &p) in pts.iter().enumerate() {
            let c = cell(p);
            items[at[c] as usize] = i as u32;
            at[c] += 1;
        }
        Grid { pts: pts.to_vec(), x0, y0, size, nx, ny, start, items }
    }

    pub fn len(&self) -> usize {
        self.pts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pts.is_empty()
    }

    pub fn point(&self, i: usize) -> (f64, f64) {
        self.pts[i]
    }

    fn bucket(&self, q: (f64, f64)) -> (i64, i64) {
        (
            (((q.0 - self.x0) / self.size).floor() as i64).clamp(0, self.nx - 1),
            (((q.1 - self.y0) / self.size).floor() as i64).clamp(0, self.ny - 1),
        )
    }

    /// Visit the buckets of ring `r` around `(cx, cy)`.
    fn ring(&self, cx: i64, cy: i64, r: i64, mut f: impl FnMut(&[u32])) {
        for by in (cy - r).max(0)..=(cy + r).min(self.ny - 1) {
            for bx in (cx - r).max(0)..=(cx + r).min(self.nx - 1) {
                if r > 0 && (bx - cx).abs() != r && (by - cy).abs() != r {
                    continue; // inside the ring, already seen
                }
                let c = (by * self.nx + bx) as usize;
                f(&self.items[self.start[c] as usize..self.start[c + 1] as usize]);
            }
        }
    }

    /// The `k` nearest points within `max`, as (distance, index), closest
    /// first; ties by index, like a stable sort by (distance, index).
    pub fn knn(&self, q: (f64, f64), k: usize, max: f64) -> Vec<(f64, u32)> {
        let mut best: Vec<(f64, u32)> = Vec::with_capacity(k + 1);
        let (cx, cy) = self.bucket(q);
        let m2 = if max.is_finite() { max * max } else { f64::MAX };
        let reach = self.nx.max(self.ny);
        for r in 0..=reach {
            // everything in this ring and beyond is at least this far away
            let d = (r - 1).max(0) as f64 * self.size;
            if d > max || (best.len() == k && d * d > best[k - 1].0) {
                break;
            }
            self.ring(cx, cy, r, |ids| {
                for &i in ids {
                    let p = self.pts[i as usize];
                    let (dx, dy) = (p.0 - q.0, p.1 - q.1);
                    let d2 = dx * dx + dy * dy;
                    if d2 > m2 {
                        continue;
                    }
                    let at = best.partition_point(|&(b, j)| (b, j) < (d2, i));
                    if at < k {
                        best.insert(at, (d2, i));
                        best.truncate(k);
                    }
                }
            });
        }
        best.into_iter().map(|(d2, i)| (d2.sqrt(), i)).collect()
    }

    /// The nearest point, or None if there is none.
    pub fn nearest(&self, q: (f64, f64)) -> Option<(f64, u32)> {
        self.knn(q, 1, f64::INFINITY).into_iter().next()
    }

    /// All index pairs i < j whose points are at most `r` apart
    /// (`cKDTree.query_pairs`), in ascending order.
    pub fn pairs(&self, r: f64) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        let reach = (r / self.size).ceil() as i64 + 1;
        let r2 = r * r;
        for i in 0..self.pts.len() {
            let q = self.pts[i];
            let (cx, cy) = self.bucket(q);
            for ring in 0..=reach {
                self.ring(cx, cy, ring, |ids| {
                    for &j in ids {
                        if (j as usize) <= i {
                            continue;
                        }
                        let p = self.pts[j as usize];
                        let (dx, dy) = (p.0 - q.0, p.1 - q.1);
                        if dx * dx + dy * dy <= r2 {
                            out.push((i as u32, j));
                        }
                    }
                });
            }
        }
        out.sort_unstable();
        out
    }
}
