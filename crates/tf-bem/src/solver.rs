//! Collocation BEM for a set of conductors in free space (potential → 0 at infinity).
//!
//! Unknowns: piecewise-constant u = σ/ε0 (V/m) on each panel. Collocation at panel
//! centroids gives  Σ_j K_ij u_j = V_i,  K_ij = (1/4π) ∫_T_j dA/|c_i − r'|.
//! Floating conductors add their unknown potential V_f and the constraint of zero net
//! charge. One basis solution per non-floating conductor: 1 V on it, 0 V on the others.

use crate::harmonics::{n_coeffs, panel_moments, Expansion};
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
}

#[derive(Debug, Clone, Copy)]
pub enum Stage {
    Assembling,
    Factoring,
    Solving,
}

pub fn solve(problem: &Problem, progress: &(dyn Fn(Stage) + Sync)) -> Result<Solution, BemError> {
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

    progress(Stage::Assembling);
    let t0 = Instant::now();
    let panels = &problem.panels;
    let cond = &problem.panel_conductor;
    let mut a = vec![0.0f64; m * m];
    a.par_chunks_mut(m).enumerate().for_each(|(j, col)| {
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

    progress(Stage::Factoring);
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

    progress(Stage::Solving);
    let t2 = Instant::now();
    let nb = basis.len();
    let mut rhs = Mat::<f64>::from_fn(m, nb, |i, b| if i < n && cond[i] == basis[b] { 1.0 } else { 0.0 });
    let lu = lu.as_ref();
    lu_solve::solve_in_place(lu, lu, p, rhs.as_mut(), par, MemStack::new(&mut MemBuffer::new(lu_solve::solve_in_place_scratch::<usize, f64>(m, nb, par))));
    let t_solve = t2.elapsed().as_secs_f64();
    drop(a);

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
    Ok(Solution { basis, u, charge, potential, n_unknowns: m, timings: Timings { assemble: t_assemble, factor: t_factor, solve: t_solve } })
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
    let mut g = vec![0.0; panels.len() * nl];
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
