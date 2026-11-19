//! Collocation BEM for a set of conductors in free space (potential → 0 at infinity).
//!
//! Unknowns: piecewise-constant u = σ/ε0 (V/m) on each panel. Collocation at panel
//! centroids gives  Σ_j K_ij u_j = V_i,  K_ij = (1/4π) ∫_T_j dA/|c_i − r'|.
//! Floating conductors add their unknown potential V_f and the constraint of zero net
//! charge. One basis solution per non-floating conductor: 1 V on it, 0 V on the others.

use crate::harmonics::{n_coeffs, panel_moments, Expansion};
use crate::hmatrix::{BlockJacobi, HMatrix, HParams, HStats};
use crate::iterative::{gmres, GmresParams, GmresStats};
use crate::panel::{closest_point, norm, single_layer, single_layer_grad, sub, Panel, V3};
use faer::dyn_stack::{MemBuffer, MemStack};
use faer::linalg::lu::partial_pivoting::{factor as lu_factor, solve as lu_solve};
use faer::{Mat, MatMut};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::time::Instant;

pub const EPS0: f64 = 8.854_187_8188e-12;
const INV_4PI: f64 = 1.0 / (4.0 * std::f64::consts::PI);

#[derive(Debug, thiserror::Error)]
pub enum BemError {
    #[error("{0}")]
    Invalid(String),
    #[error("stopped")]
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conductor {
    pub name: String,
    pub floating: bool,
}

pub struct Problem {
    pub panels: Vec<Panel>,
    /// Conductor index of each panel.
    pub panel_conductor: Vec<usize>,
    pub conductors: Vec<Conductor>,
}

impl Problem {
    /// Build panels from an indexed triangle mesh (`nodes` xyz in m).
    pub fn from_mesh(nodes: &[f64], triangles: &[u32], tri_conductor: Vec<usize>, conductors: Vec<Conductor>) -> Result<Problem, BemError> {
        let n_nodes = nodes.len() / 3;
        if let Some(&bad) = triangles.iter().find(|&&i| i as usize >= n_nodes) {
            return Err(BemError::Invalid(format!("node index {bad} out of range ({n_nodes} nodes)")));
        }
        if tri_conductor.len() != triangles.len() / 3 || tri_conductor.iter().any(|&c| c >= conductors.len()) {
            return Err(BemError::Invalid("triangle conductor indices do not match the mesh".into()));
        }
        let p = |i: u32| -> V3 {
            let i = 3 * i as usize;
            [nodes[i], nodes[i + 1], nodes[i + 2]]
        };
        let mut panels = Vec::with_capacity(triangles.len() / 3);
        // duplicate panels make the collocation matrix exactly singular
        let mut seen = std::collections::HashMap::new();
        for (t, tri) in triangles.chunks_exact(3).enumerate() {
            let mut k = [tri[0], tri[1], tri[2]];
            k.sort_unstable();
            if let Some(first) = seen.insert(k, t) {
                return Err(BemError::Invalid(format!("triangles {first} and {t} are duplicates; the mesh is defective")));
            }
            let panel = Panel::new(p(tri[0]), p(tri[1]), p(tri[2])).ok_or_else(|| BemError::Invalid(format!("triangle {t} is degenerate (zero area)")))?;
            panels.push(panel);
        }
        if tri_conductor.len() != panels.len() {
            return Err(BemError::Invalid("one conductor index per triangle required".into()));
        }
        if let Some(&c) = tri_conductor.iter().find(|&&c| c >= conductors.len()) {
            return Err(BemError::Invalid(format!("conductor index {c} out of range")));
        }
        Ok(Problem { panels, panel_conductor: tri_conductor, conductors })
    }

    /// Distance from `x` to the nearest panel.
    pub fn distance_to_surface(&self, x: V3) -> f64 {
        self.panels.par_iter().map(|p| norm(sub(x, closest_point(p, x)))).reduce(|| f64::INFINITY, f64::min)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Timings {
    pub assemble: f64,
    pub factor: f64,
    pub solve: f64,
}

pub struct Solution {
    /// Conductor driven at 1 V in each basis solution.
    pub basis: Vec<usize>,
    /// u = σ/ε0 (V/m) per panel, one vector per basis solution.
    pub u: Vec<Vec<f64>>,
    /// charge[b][k]: charge (C) on conductor k in basis solution b (= capacitance matrix, F).
    pub charge: Vec<Vec<f64>>,
    /// potential[b][k]: potential (V) of conductor k in basis solution b (floating ones solved for).
    pub potential: Vec<Vec<f64>>,
    pub n_unknowns: usize,
    pub timings: Timings,
    pub stats: SolverStats,
}

/// How the collocation system is solved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    /// Full matrix and LU factorization: exact, O(N²) memory, O(N³) time.
    #[default]
    Dense,
    /// H-matrix (ACA-compressed far field) and preconditioned GMRES.
    Compressed,
}

#[derive(Debug, Clone, Copy)]
pub struct SolveOptions {
    pub method: Method,
    pub h: HParams,
    pub gmres: GmresParams,
    /// Block size of the block-Jacobi preconditioner.
    pub precond_block: usize,
}

impl Default for SolveOptions {
    fn default() -> Self {
        SolveOptions { method: Method::Dense, h: HParams::default(), gmres: GmresParams::default(), precond_block: 256 }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SolverStats {
    pub method: Method,
    /// Bytes held by the system matrix (dense or compressed).
    pub matrix_bytes: f64,
    /// Compressed storage / dense storage.
    #[serde(default)]
    pub compression: Option<f64>,
    #[serde(default)]
    pub max_rank: Option<usize>,
    /// GMRES iterations per right-hand side.
    #[serde(default)]
    pub iterations: Vec<usize>,
    /// Final relative residuals.
    #[serde(default)]
    pub residual: Vec<f64>,
    /// Estimated relative residual per right-hand side after each GMRES iteration.
    #[serde(default)]
    pub history: Vec<Vec<f64>>,
}

/// Progress reports from a solve.
#[derive(Debug, Clone, Copy)]
pub enum Progress<'a> {
    Stage(Stage),
    /// After each GMRES iteration: estimated relative residual of every right-hand side.
    Iteration { iteration: usize, residuals: &'a [f64] },
}

#[derive(Debug, Clone, Copy)]
pub enum Stage {
    Assembling,
    Factoring,
    Solving,
}

/// Solve with the given method (see [`SolveOptions`]).
pub fn solve_with(problem: &Problem, opts: &SolveOptions, progress: &(dyn Fn(Progress) + Sync)) -> Result<Solution, BemError> {
    solve_with_stop(problem, opts, progress, &|| false)
}

/// [`solve_with`], polling `stop` between stages, while assembling and after every GMRES
/// iteration (an LU factorization runs to the end); returns [`BemError::Stopped`] when
/// it returns true.
pub fn solve_with_stop(
    problem: &Problem,
    opts: &SolveOptions,
    progress: &(dyn Fn(Progress) + Sync),
    stop: &(dyn Fn() -> bool + Sync),
) -> Result<Solution, BemError> {
    match opts.method {
        Method::Dense => solve_dense(problem, progress, stop),
        Method::Compressed => solve_compressed(problem, opts, progress, stop),
    }
}

/// Dense collocation matrix and in-place LU.
pub fn solve(problem: &Problem, progress: &(dyn Fn(Progress) + Sync)) -> Result<Solution, BemError> {
    solve_dense(problem, progress, &|| false)
}

fn solve_dense(problem: &Problem, progress: &(dyn Fn(Progress) + Sync), stop: &(dyn Fn() -> bool + Sync)) -> Result<Solution, BemError> {
    let n = problem.panels.len();
    let nc = problem.conductors.len();
    if n == 0 {
        return Err(BemError::Invalid("no panels".into()));
    }
    let mut float_idx = vec![None; nc];
    let mut nf = 0;
    for (k, c) in problem.conductors.iter().enumerate() {
        if c.floating {
            float_idx[k] = Some(nf);
            nf += 1;
        }
    }
    let basis: Vec<usize> = (0..nc).filter(|&k| !problem.conductors[k].floating).collect();
    if basis.is_empty() {
        return Err(BemError::Invalid("every conductor is floating; at least one needs a fixed potential".into()));
    }
    for k in 0..nc {
        if !problem.panel_conductor.contains(&k) {
            return Err(BemError::Invalid(format!("conductor '{}' has no panels", problem.conductors[k].name)));
        }
    }
    let m = n + nf;
    // typical panel size, to keep the floating-conductor rows/columns of similar magnitude
    let lc = problem.panels.iter().map(|p| p.area.sqrt()).sum::<f64>() / n as f64;

    progress(Progress::Stage(Stage::Assembling));
    let t0 = Instant::now();
    let panels = &problem.panels;
    let cond = &problem.panel_conductor;
    // m² doubles: refuse (rather than abort) when that cannot be allocated, e.g. beyond
    // 2 GiB on 32-bit WebAssembly
    let mut a: Vec<f64> = Vec::new();
    m.checked_mul(m)
        .filter(|&mm| (mm as u64) * 8 <= isize::MAX as u64)
        .and_then(|mm| a.try_reserve_exact(mm).ok().map(|_| mm))
        .map(|mm| a.resize(mm, 0.0))
        .ok_or_else(|| BemError::Invalid(format!("{m} unknowns: the dense matrix ({:.1} GB) does not fit in memory; use the compressed solver", 8.0 * (m as f64).powi(2) / 1e9)))?;
    let skipped = std::sync::atomic::AtomicBool::new(false);
    a.par_chunks_mut(m).enumerate().for_each(|(j, col)| {
        if stop() {
            skipped.store(true, std::sync::atomic::Ordering::Relaxed);
            return;
        }
        if j < n {
            let pj = &panels[j];
            for (i, pi) in panels.iter().enumerate() {
                col[i] = INV_4PI * single_layer(pj, pi.centroid);
            }
            if let Some(f) = float_idx[cond[j]] {
                col[n + f] = pj.area / lc; // zero net charge on floating conductor f
            }
        } else {
            let f = j - n;
            for i in 0..n {
                if float_idx[cond[i]] == Some(f) {
                    col[i] = -lc; // unknown potential, scaled by lc
                }
            }
        }
    });
    let t_assemble = t0.elapsed().as_secs_f64();
    // a skipped column leaves the matrix incomplete, even if the flag were cleared again
    if skipped.load(std::sync::atomic::Ordering::Relaxed) || stop() {
        return Err(BemError::Stopped);
    }

    progress(Progress::Stage(Stage::Factoring));
    let t1 = Instant::now();
    // LU in place (L strictly below the diagonal with unit diagonal, U on and above it),
    // so peak memory is one m×m matrix
    let par = faer::get_global_parallelism();
    let mut perm = vec![0usize; m];
    let mut perm_inv = vec![0usize; m];
    let mut lu = MatMut::from_column_major_slice_mut(&mut a, m, m);
    let (_, p) = lu_factor::lu_in_place(
        lu.as_mut(),
        &mut perm,
        &mut perm_inv,
        par,
        MemStack::new(&mut MemBuffer::new(lu_factor::lu_in_place_scratch::<usize, f64>(m, m, par, Default::default()))),
        Default::default(),
    );
    let t_factor = t1.elapsed().as_secs_f64();
    if stop() {
        return Err(BemError::Stopped);
    }

    progress(Progress::Stage(Stage::Solving));
    let t2 = Instant::now();
    let nb = basis.len();
    let mut rhs = Mat::<f64>::from_fn(m, nb, |i, b| if i < n && cond[i] == basis[b] { 1.0 } else { 0.0 });
    let lu = lu.as_ref();
    lu_solve::solve_in_place(lu, lu, p, rhs.as_mut(), par, MemStack::new(&mut MemBuffer::new(lu_solve::solve_in_place_scratch::<usize, f64>(m, nb, par))));
    let t_solve = t2.elapsed().as_secs_f64();
    drop(a);
    // a (numerically) singular system gives no error from the LU, only non-finite values
    if (0..m).any(|i| (0..basis.len()).any(|b| !rhs[(i, b)].is_finite())) {
        return Err(BemError::Invalid("the system is singular (non-finite solution): check the mesh for coincident panels".into()));
    }

    let mut u = Vec::with_capacity(basis.len());
    let mut charge = Vec::with_capacity(basis.len());
    let mut potential = Vec::with_capacity(basis.len());
    for (b, &driven) in basis.iter().enumerate() {
        let col: Vec<f64> = (0..n).map(|i| rhs[(i, b)]).collect();
        let mut q = vec![0.0; nc];
        for i in 0..n {
            q[cond[i]] += EPS0 * col[i] * panels[i].area;
        }
        let v: Vec<f64> = (0..nc)
            .map(|k| match float_idx[k] {
                Some(f) => rhs[(n + f, b)] * lc,
                None => (k == driven) as u8 as f64,
            })
            .collect();
        u.push(col);
        charge.push(q);
        potential.push(v);
    }
    let stats = SolverStats { method: Method::Dense, matrix_bytes: 8.0 * (m as f64).powi(2), ..Default::default() };
    Ok(Solution { basis, u, charge, potential, n_unknowns: m, timings: Timings { assemble: t_assemble, factor: t_factor, solve: t_solve }, stats })
}

/// Potential (V) and its gradient (V/m) at each point from the density u = σ/ε0.
pub fn evaluate(panels: &[Panel], u: &[f64], points: &[V3]) -> Vec<(f64, V3)> {
    points
        .par_iter()
        .map(|&x| {
            let mut phi = 0.0;
            let mut g = [0.0; 3];
            for (p, &uj) in panels.iter().zip(u) {
                let (s, gs) = single_layer_grad(p, x);
                phi += uj * s;
                for k in 0..3 {
                    g[k] += uj * gs[k];
                }
            }
            (phi * INV_4PI, [g[0] * INV_4PI, g[1] * INV_4PI, g[2] * INV_4PI])
        })
        .collect()
}

/// Per-panel moment table (n_panels × (lmax+1)²) for expansions about `center`.
pub fn moment_table(panels: &[Panel], center: V3, r0: f64, lmax: usize) -> Vec<f64> {
    let nl = n_coeffs(lmax);
    let len = panels.len().checked_mul(nl).filter(|&n| (n as u64) * 8 <= isize::MAX as u64).expect("moment table larger than the address space; lower l_max");
    let mut g = vec![0.0; len];
    g.par_chunks_mut(nl).zip(panels.par_iter()).for_each(|(row, p)| panel_moments(p, center, r0, lmax, row));
    g
}

/// Solid-harmonic expansion of the potential of density u, from a `moment_table`.
pub fn expand(table: &[f64], u: &[f64], center: V3, r0: f64, lmax: usize) -> Expansion {
    let nl = n_coeffs(lmax);
    let mut coeffs = vec![0.0; nl];
    for (row, &uj) in table.chunks_exact(nl).zip(u) {
        for (c, g) in coeffs.iter_mut().zip(row) {
            *c += uj * g;
        }
    }
    Expansion { lmax, center, r0, coeffs }
}

/// H-matrix + GMRES. Every conductor (floating ones included) is solved at 1 V with the
/// others at 0 V; floating potentials then follow from zero net charge by superposition,
/// so the iterative solver only ever sees the plain collocation matrix.
fn solve_compressed(
    problem: &Problem,
    opts: &SolveOptions,
    progress: &(dyn Fn(Progress) + Sync),
    stop: &(dyn Fn() -> bool + Sync),
) -> Result<Solution, BemError> {
    let n = problem.panels.len();
    let nc = problem.conductors.len();
    let panels = &problem.panels;
    let cond = &problem.panel_conductor;
    let basis: Vec<usize> = (0..nc).filter(|&k| !problem.conductors[k].floating).collect();
    let floating: Vec<usize> = (0..nc).filter(|&k| problem.conductors[k].floating).collect();
    if basis.is_empty() {
        return Err(BemError::Invalid("every conductor is floating; at least one needs a fixed potential".into()));
    }
    for k in 0..nc {
        if !cond.contains(&k) {
            return Err(BemError::Invalid(format!("conductor '{}' has no panels", problem.conductors[k].name)));
        }
    }

    progress(Progress::Stage(Stage::Assembling));
    let t0 = Instant::now();
    let h = HMatrix::build_with_stop(panels, &opts.h, stop).ok_or(BemError::Stopped)?;
    let t_assemble = t0.elapsed().as_secs_f64();

    if stop() {
        return Err(BemError::Stopped);
    }
    progress(Progress::Stage(Stage::Factoring));
    let t1 = Instant::now();
    let pre = BlockJacobi::build(panels, &h.tree, opts.precond_block);
    let t_factor = t1.elapsed().as_secs_f64();

    progress(Progress::Stage(Stage::Solving));
    let t2 = Instant::now();
    let perm = &h.tree.perm;
    let mut b = vec![0.0; n * nc];
    for c in 0..nc {
        for (p, &i) in perm.iter().enumerate() {
            if cond[i] == c {
                b[c * n + p] = 1.0;
            }
        }
    }
    let mut x = vec![0.0; n * nc];
    let op = |xin: &[f64], y: &mut [f64], k: usize| h.matvec(xin, y, k);
    let prec = |v: &mut [f64], k: usize| pre.apply(v, n, k);
    let on_iter = |iteration: usize, residuals: &[f64]| progress(Progress::Iteration { iteration, residuals });
    let gs: GmresStats = gmres(n, nc, &b, &mut x, &op, &prec, &opts.gmres, &on_iter, stop);
    if gs.stopped {
        return Err(BemError::Stopped);
    }
    if !gs.converged {
        return Err(BemError::Invalid(format!(
            "GMRES did not converge (residuals {:?} after {:?} iterations)",
            gs.residual, gs.iterations
        )));
    }
    // back to panel order: per-conductor unit solutions
    let unit: Vec<Vec<f64>> = (0..nc)
        .map(|c| {
            let mut u = vec![0.0; n];
            for (p, &i) in perm.iter().enumerate() {
                u[i] = x[c * n + p];
            }
            u
        })
        .collect();
    let net = |u: &[f64], k: usize| -> f64 { (0..n).filter(|&i| cond[i] == k).map(|i| u[i] * panels[i].area).sum() };
    // floating potentials: Σ_f V_f Q_g(u_f) = −Q_g(u_b) for every floating g
    let cmat: Vec<Vec<f64>> = floating.iter().map(|&g| floating.iter().map(|&f| net(&unit[f], g)).collect()).collect();
    let mut u = Vec::with_capacity(basis.len());
    let mut charge = Vec::with_capacity(basis.len());
    let mut potential = Vec::with_capacity(basis.len());
    for &bk in &basis {
        let rhs: Vec<f64> = floating.iter().map(|&g| -net(&unit[bk], g)).collect();
        let vf = solve_small(cmat.clone(), rhs);
        let mut ub = unit[bk].clone();
        for (fi, &f) in floating.iter().enumerate() {
            for (a, x) in ub.iter_mut().zip(&unit[f]) {
                *a += vf[fi] * x;
            }
        }
        let q: Vec<f64> = (0..nc).map(|k| EPS0 * net(&ub, k)).collect();
        let v: Vec<f64> = (0..nc)
            .map(|k| match floating.iter().position(|&f| f == k) {
                Some(fi) => vf[fi],
                None => (k == bk) as u8 as f64,
            })
            .collect();
        u.push(ub);
        charge.push(q);
        potential.push(v);
    }
    let t_solve = t2.elapsed().as_secs_f64();
    let hs: &HStats = &h.stats;
    let stats = SolverStats {
        method: Method::Compressed,
        matrix_bytes: hs.bytes,
        compression: Some(hs.compression),
        max_rank: Some(hs.max_rank),
        iterations: gs.iterations,
        residual: gs.residual,
        history: gs.history,
    };
    Ok(Solution { basis, u, charge, potential, n_unknowns: n, timings: Timings { assemble: t_assemble, factor: t_factor, solve: t_solve }, stats })
}

/// Gaussian elimination with partial pivoting for a small dense system.
fn solve_small(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Vec<f64> {
    let n = b.len();
    for c in 0..n {
        let p = (c..n).max_by(|&i, &j| a[i][c].abs().partial_cmp(&a[j][c].abs()).unwrap()).unwrap();
        a.swap(c, p);
        b.swap(c, p);
        for r in (c + 1)..n {
            let f = a[r][c] / a[c][c];
            for k in c..n {
                a[r][k] -= f * a[c][k];
            }
            b[r] -= f * b[c];
        }
    }
    let mut x = vec![0.0; n];
    for r in (0..n).rev() {
        let s: f64 = ((r + 1)..n).map(|k| a[r][k] * x[k]).sum();
        x[r] = (b[r] - s) / a[r][r];
    }
    x
}
