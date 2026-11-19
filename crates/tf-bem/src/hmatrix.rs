//! Hierarchical-matrix (H-matrix) representation of the collocation matrix
//! K_ij = (1/4π) ∫_T_j dA/|c_i − r'|, so that large problems need neither O(N²) memory nor
//! an O(N³) factorization.
//!
//! Panels are ordered along a cluster tree (recursive bisection of the centroids). A pair of
//! clusters (τ, σ) whose bounding boxes satisfy  min(diam τ, diam σ) ≤ η · dist(τ, σ)  is
//! "admissible": the kernel is smooth there and the block is replaced by a low-rank product
//! U Vᵀ built with adaptive cross approximation (ACA, partial pivoting) from a few rows and
//! columns. Remaining (near-field) blocks between leaf clusters are stored densely.

use crate::panel::{single_layer, Panel, V3};
use rayon::prelude::*;
use serde::Serialize;
use std::ops::Range;

const INV_4PI: f64 = 1.0 / (4.0 * std::f64::consts::PI);

#[derive(Debug, Clone, Copy)]
pub struct HParams {
    /// Admissibility parameter: larger compresses more blocks at higher rank.
    pub eta: f64,
    /// Clusters with at most this many panels are not split further.
    pub leaf_size: usize,
    /// Relative accuracy of each low-rank block.
    pub aca_eps: f64,
}

impl Default for HParams {
    fn default() -> Self {
        HParams { eta: 2.0, leaf_size: 64, aca_eps: 1e-6 }
    }
}

#[derive(Debug, Clone)]
pub struct Cluster {
    /// Range of positions in tree order.
    pub range: Range<usize>,
    pub lo: V3,
    pub hi: V3,
    pub children: Option<(usize, usize)>,
}

impl Cluster {
    fn diam(&self) -> f64 {
        ((0..3).map(|k| (self.hi[k] - self.lo[k]).powi(2)).sum::<f64>()).sqrt()
    }
    fn dist(&self, o: &Cluster) -> f64 {
        (0..3).map(|k| (o.lo[k] - self.hi[k]).max(self.lo[k] - o.hi[k]).max(0.0).powi(2)).sum::<f64>().sqrt()
    }
    pub fn len(&self) -> usize {
        self.range.len()
    }
}

/// Cluster tree over panels; `perm[p]` is the panel index at tree position p.
pub struct ClusterTree {
    pub nodes: Vec<Cluster>,
    pub perm: Vec<usize>,
}

impl ClusterTree {
    pub fn build(panels: &[Panel], leaf_size: usize) -> ClusterTree {
        let mut perm: Vec<usize> = (0..panels.len()).collect();
        let mut nodes = Vec::new();
        Self::split(panels, &mut perm, 0, panels.len(), leaf_size.max(2), &mut nodes);
        ClusterTree { nodes, perm }
    }

    fn split(panels: &[Panel], perm: &mut [usize], start: usize, end: usize, leaf: usize, nodes: &mut Vec<Cluster>) -> usize {
        // bounding box of the panels themselves (not just centroids)
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        let (mut clo, mut chi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for &p in &perm[start..end] {
            for v in &panels[p].v {
                for k in 0..3 {
                    lo[k] = lo[k].min(v[k]);
                    hi[k] = hi[k].max(v[k]);
                }
            }
            let c = panels[p].centroid;
            for k in 0..3 {
                clo[k] = clo[k].min(c[k]);
                chi[k] = chi[k].max(c[k]);
            }
        }
        let id = nodes.len();
        nodes.push(Cluster { range: start..end, lo, hi, children: None });
        if end - start > leaf {
            // bisect along the longest axis of the centroid box, at the median
            let axis = (0..3).max_by(|&a, &b| (chi[a] - clo[a]).partial_cmp(&(chi[b] - clo[b])).unwrap()).unwrap();
            let mid = start + (end - start) / 2;
            perm[start..end].select_nth_unstable_by(mid - start, |&a, &b| panels[a].centroid[axis].partial_cmp(&panels[b].centroid[axis]).unwrap());
            let a = Self::split(panels, perm, start, mid, leaf, nodes);
            let b = Self::split(panels, perm, mid, end, leaf, nodes);
            nodes[id].children = Some((a, b));
        }
        id
    }

    /// Clusters covering all panels with at most `max` panels each (as large as possible).
    pub fn partition(&self, max: usize) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        let mut stack = vec![0];
        while let Some(i) = stack.pop() {
            let c = &self.nodes[i];
            match c.children {
                Some((a, b)) if c.len() > max => {
                    stack.push(b);
                    stack.push(a);
                }
                _ => out.push(c.range.clone()),
            }
        }
        out.sort_by_key(|r| r.start);
        out
    }
}

pub enum BlockData {
    /// m×n, column-major.
    Dense(Vec<f64>),
    /// U (m×k) and V (n×k), column-major: block = U Vᵀ.
    LowRank { u: Vec<f64>, v: Vec<f64>, k: usize },
}

pub struct Block {
    pub rows: Range<usize>,
    pub cols: Range<usize>,
    pub data: BlockData,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct HStats {
    pub n: usize,
    pub dense_blocks: usize,
    pub lowrank_blocks: usize,
    pub max_rank: usize,
    /// Stored f64 values / N².
    pub compression: f64,
    pub bytes: f64,
}

/// The collocation matrix in tree order.
pub struct HMatrix {
    pub tree: ClusterTree,
    pub blocks: Vec<Block>,
    pub stats: HStats,
    /// Row chunks (cluster ranges) for the parallel product, with the blocks touching each.
    chunks: Vec<(Range<usize>, Vec<usize>)>,
}

/// Kernel entry for tree positions (row = collocation point, col = source panel).
#[inline]
fn entry(panels: &[Panel], perm: &[usize], i: usize, j: usize) -> f64 {
    INV_4PI * single_layer(&panels[perm[j]], panels[perm[i]].centroid)
}

/// ACA with partial pivoting. Returns None if the block does not compress well.
fn aca(panels: &[Panel], perm: &[usize], rows: Range<usize>, cols: Range<usize>, eps: f64) -> Option<(Vec<f64>, Vec<f64>, usize)> {
    let (m, n) = (rows.len(), cols.len());
    let max_rank = (m.min(n) / 2).max(1);
    let mut u: Vec<f64> = Vec::new();
    let mut v: Vec<f64> = Vec::new();
    let mut used_row = vec![false; m];
    let mut norm2 = 0.0f64;
    let mut i_star = 0;
    let mut k = 0;
    let mut zero_rows = 0;
    loop {
        // residual row i*
        let mut row: Vec<f64> = cols.clone().map(|j| entry(panels, perm, rows.start + i_star, j)).collect();
        for l in 0..k {
            let ul = u[l * m + i_star];
            if ul != 0.0 {
                for (jj, r) in row.iter_mut().enumerate() {
                    *r -= ul * v[l * n + jj];
                }
            }
        }
        used_row[i_star] = true;
        let (j_star, piv) = row.iter().enumerate().fold((0, 0.0f64), |b, (j, &x)| if x.abs() > b.1.abs() { (j, x) } else { b });
        if piv.abs() <= 1e-300 {
            // this row is already reproduced; try another one
            zero_rows += 1;
            match (0..m).find(|&i| !used_row[i]) {
                Some(i) if zero_rows < 8 => {
                    i_star = i;
                    continue;
                }
                _ => break,
            }
        }
        let vk: Vec<f64> = row.iter().map(|x| x / piv).collect();
        let mut uk: Vec<f64> = rows.clone().map(|i| entry(panels, perm, i, cols.start + j_star)).collect();
        for l in 0..k {
            let vl = v[l * n + j_star];
            if vl != 0.0 {
                for (ii, x) in uk.iter_mut().enumerate() {
                    *x -= vl * u[l * m + ii];
                }
            }
        }
        // Frobenius norm of the approximation, updated incrementally
        let nu2: f64 = uk.iter().map(|x| x * x).sum();
        let nv2: f64 = vk.iter().map(|x| x * x).sum();
        let mut cross = 0.0;
        for l in 0..k {
            let du: f64 = (0..m).map(|i| uk[i] * u[l * m + i]).sum();
            let dv: f64 = (0..n).map(|j| vk[j] * v[l * n + j]).sum();
            cross += du * dv;
        }
        norm2 += nu2 * nv2 + 2.0 * cross;
        u.extend_from_slice(&uk);
        v.extend_from_slice(&vk);
        k += 1;
        if (nu2 * nv2).sqrt() <= eps * norm2.max(0.0).sqrt() {
            break;
        }
        if k >= max_rank {
            return None;
        }
        // next pivot row: largest entry of the new column among unused rows
        match (0..m).filter(|&i| !used_row[i]).max_by(|&a, &b| uk[a].abs().partial_cmp(&uk[b].abs()).unwrap()) {
            Some(i) => i_star = i,
            None => break,
        }
    }
    if k == 0 {
        // block is numerically zero
        return Some((vec![0.0; m], vec![0.0; n], 1));
    }
    // the factors grew by doubling: release the spare capacity (~25 % of the H-matrix)
    u.shrink_to_fit();
    v.shrink_to_fit();
    Some((u, v, k))
}

impl HMatrix {
    pub fn build(panels: &[Panel], p: &HParams) -> HMatrix {
        Self::build_with_stop(panels, p, &|| false).expect("not stopped")
    }

    /// [`HMatrix::build`], polling `stop` before each block; None if it returned true.
    pub fn build_with_stop(panels: &[Panel], p: &HParams, stop: &(dyn Fn() -> bool + Sync)) -> Option<HMatrix> {
        let tree = ClusterTree::build(panels, p.leaf_size);
        // block cluster tree
        let mut leaves: Vec<(usize, usize, bool)> = Vec::new(); // (row node, col node, admissible)
        let mut stack = vec![(0usize, 0usize)];
        while let Some((t, s)) = stack.pop() {
            let (ct, cs) = (&tree.nodes[t], &tree.nodes[s]);
            if ct.diam().min(cs.diam()) <= p.eta * ct.dist(cs) {
                leaves.push((t, s, true));
                continue;
            }
            match (ct.children, cs.children) {
                (Some((a, b)), Some((c, d))) => {
                    stack.extend([(a, c), (a, d), (b, c), (b, d)]);
                }
                (Some((a, b)), None) => stack.extend([(a, s), (b, s)]),
                (None, Some((c, d))) => stack.extend([(t, c), (t, d)]),
                (None, None) => leaves.push((t, s, false)),
            }
        }
        let perm = &tree.perm;
        let dense = |rows: Range<usize>, cols: Range<usize>| -> Vec<f64> {
            let mut d = Vec::with_capacity(rows.len() * cols.len());
            for j in cols.clone() {
                for i in rows.clone() {
                    d.push(entry(panels, perm, i, j));
                }
            }
            d
        };
        let skipped = std::sync::atomic::AtomicBool::new(false);
        let mut blocks: Vec<Block> = leaves
            .par_iter()
            .map(|&(t, s, adm)| {
                let (rows, cols) = (tree.nodes[t].range.clone(), tree.nodes[s].range.clone());
                let data = if stop() {
                    // a placeholder: the matrix is discarded below
                    skipped.store(true, std::sync::atomic::Ordering::Relaxed);
                    BlockData::Dense(Vec::new())
                } else if adm {
                    match aca(panels, perm, rows.clone(), cols.clone(), p.aca_eps) {
                        Some((u, v, k)) => BlockData::LowRank { u, v, k },
                        None => BlockData::Dense(dense(rows.clone(), cols.clone())),
                    }
                } else {
                    BlockData::Dense(dense(rows.clone(), cols.clone()))
                };
                Block { rows, cols, data }
            })
            .collect();
        if skipped.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        // larger blocks first: better load balance in the parallel matvec
        blocks.sort_by_key(|b| std::cmp::Reverse(b.rows.len() as u64 * b.cols.len() as u64));
        let n = panels.len();
        let mut stats = HStats { n, ..Default::default() };
        let mut stored = 0usize;
        for b in &blocks {
            match &b.data {
                BlockData::Dense(d) => {
                    stats.dense_blocks += 1;
                    stored += d.len();
                }
                BlockData::LowRank { u, v, k } => {
                    stats.lowrank_blocks += 1;
                    stats.max_rank = stats.max_rank.max(*k);
                    stored += u.len() + v.len();
                }
            }
        }
        stats.compression = stored as f64 / (n as f64 * n as f64);
        stats.bytes = 8.0 * stored as f64;
        // row chunks aligned with the cluster tree: each block's row range is inside one chunk
        // or a union of whole chunks, so chunks can be processed independently
        let target = (n / (8 * rayon::current_num_threads().max(1))).max(p.leaf_size);
        let ranges = tree.partition(target);
        let mut chunks: Vec<(Range<usize>, Vec<usize>)> = ranges.into_iter().map(|r| (r, Vec::new())).collect();
        let starts: Vec<usize> = chunks.iter().map(|c| c.0.start).collect();
        for (bi, b) in blocks.iter().enumerate() {
            let first = starts.partition_point(|&s| s <= b.rows.start) - 1;
            let mut c = first;
            while c < chunks.len() && chunks[c].0.start < b.rows.end {
                chunks[c].1.push(bi);
                c += 1;
            }
        }
        Some(HMatrix { tree, blocks, stats, chunks })
    }

    pub fn n(&self) -> usize {
        self.tree.perm.len()
    }

    /// Y = K X for `k` vectors stored column-major (N×k), in tree order.
    pub fn matvec(&self, x: &[f64], y: &mut [f64], k: usize) {
        let n = self.n();
        // 1. inner products Vᵀ X for every low-rank block (rank × k each)
        let t: Vec<Vec<f64>> = self
            .blocks
            .par_iter()
            .map(|b| match &b.data {
                BlockData::LowRank { v, k: rank, .. } => {
                    let (c0, nc) = (b.cols.start, b.cols.len());
                    let mut t = vec![0.0; rank * k];
                    for r in 0..k {
                        let xs = &x[r * n + c0..r * n + c0 + nc];
                        for l in 0..*rank {
                            t[r * rank + l] = v[l * nc..(l + 1) * nc].iter().zip(xs).map(|(a, b)| a * b).sum();
                        }
                    }
                    t
                }
                BlockData::Dense(_) => Vec::new(),
            })
            .collect();
        // 2. each row chunk accumulates its own rows: no reduction needed
        let parts: Vec<Vec<f64>> = self
            .chunks
            .par_iter()
            .map(|(rows, members)| {
                let (q0, qm) = (rows.start, rows.len());
                let mut out = vec![0.0; qm * k];
                for &bi in members {
                    let b = &self.blocks[bi];
                    let (r0, m) = (b.rows.start, b.rows.len());
                    // rows of this block that fall in the chunk
                    let lo = r0.max(q0);
                    let hi = (r0 + m).min(q0 + qm);
                    let (bo, len, oo) = (lo - r0, hi - lo, lo - q0);
                    match &b.data {
                        BlockData::Dense(d) => {
                            let (c0, nc) = (b.cols.start, b.cols.len());
                            for r in 0..k {
                                let xs = &x[r * n + c0..r * n + c0 + nc];
                                let ys = &mut out[r * qm + oo..r * qm + oo + len];
                                for (j, &xj) in xs.iter().enumerate() {
                                    if xj != 0.0 {
                                        let col = &d[j * m + bo..j * m + bo + len];
                                        for (yi, &a) in ys.iter_mut().zip(col) {
                                            *yi += a * xj;
                                        }
                                    }
                                }
                            }
                        }
                        BlockData::LowRank { u, k: rank, .. } => {
                            let tb = &t[bi];
                            for r in 0..k {
                                let ys = &mut out[r * qm + oo..r * qm + oo + len];
                                for l in 0..*rank {
                                    let tl = tb[r * rank + l];
                                    for (yi, &a) in ys.iter_mut().zip(&u[l * m + bo..l * m + bo + len]) {
                                        *yi += a * tl;
                                    }
                                }
                            }
                        }
                    }
                }
                out
            })
            .collect();
        for ((rows, _), out) in self.chunks.iter().zip(&parts) {
            let qm = rows.len();
            for r in 0..k {
                y[r * n + rows.start..r * n + rows.end].copy_from_slice(&out[r * qm..(r + 1) * qm]);
            }
        }
    }
}

/// Block-Jacobi preconditioner: exact inverses of the diagonal blocks over a partition of
/// the cluster tree into groups of at most `max` panels.
pub struct BlockJacobi {
    blocks: Vec<(Range<usize>, faer::linalg::solvers::PartialPivLu<f64>)>,
}

impl BlockJacobi {
    pub fn build(panels: &[Panel], tree: &ClusterTree, max: usize) -> BlockJacobi {
        let perm = &tree.perm;
        let blocks = tree
            .partition(max)
            .into_par_iter()
            .map(|r| {
                let m = r.len();
                let a = faer::Mat::<f64>::from_fn(m, m, |i, j| entry(panels, perm, r.start + i, r.start + j));
                (r, a.partial_piv_lu())
            })
            .collect();
        BlockJacobi { blocks }
    }

    /// x ← M⁻¹ x for `k` vectors (N×k, column-major, tree order).
    pub fn apply(&self, x: &mut [f64], n: usize, k: usize) {
        use faer::linalg::solvers::Solve;
        // each block touches a disjoint row range: split x into per-block chunks
        let parts: Vec<(usize, Vec<f64>)> = self
            .blocks
            .par_iter()
            .enumerate()
            .map(|(bi, (r, lu))| {
                let m = r.len();
                let mut rhs = faer::Mat::<f64>::from_fn(m, k, |i, c| x[c * n + r.start + i]);
                lu.solve_in_place(rhs.as_mut());
                let mut out = Vec::with_capacity(m * k);
                for c in 0..k {
                    for i in 0..m {
                        out.push(rhs[(i, c)]);
                    }
                }
                (bi, out)
            })
            .collect();
        for (bi, out) in parts {
            let r = &self.blocks[bi].0;
            let m = r.len();
            for c in 0..k {
                x[c * n + r.start..c * n + r.end].copy_from_slice(&out[c * m..(c + 1) * m]);
            }
        }
    }
}
