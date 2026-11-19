//! Restarted GMRES with right preconditioning, for several right-hand sides at once.
//!
//! The right-hand sides iterate in lockstep so that every operator application acts on a
//! block of vectors (one pass over the H-matrix data for all of them).

use rayon::prelude::*;
use serde::Serialize;

#[derive(Debug, Clone, Copy)]
pub struct GmresParams {
    /// Relative residual |b − A x| / |b| to reach.
    pub tol: f64,
    /// Krylov basis size before a restart.
    pub restart: usize,
    /// Total iterations per right-hand side.
    pub max_iter: usize,
}

impl Default for GmresParams {
    fn default() -> Self {
        GmresParams { tol: 1e-9, restart: 120, max_iter: 2000 }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct GmresStats {
    /// Iterations (operator applications) per right-hand side.
    pub iterations: Vec<usize>,
    /// Final relative residual per right-hand side (recomputed, not estimated).
    pub residual: Vec<f64>,
    pub converged: bool,
    pub restarts: usize,
    /// Estimated relative residual of every right-hand side after each iteration.
    pub history: Vec<Vec<f64>>,
    /// Stopped on request before converging.
    pub stopped: bool,
}

struct State {
    bnorm: f64,
    basis: Vec<Vec<f64>>,
    h: Vec<Vec<f64>>, // columns of the Hessenberg matrix
    cs: Vec<f64>,
    sn: Vec<f64>,
    g: Vec<f64>,
    size: usize,
    cycle_done: bool,
    converged: bool,
    iters: usize,
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Solve A X = B for `k` right-hand sides (N×k column-major). `op(x, y, m)` computes
/// Y = A X for m packed vectors; `prec(x, m)` applies M⁻¹ in place. X holds the initial
/// guess on entry and the solution on return. `stop` is polled after every iteration;
/// when it returns true the iteration ends with `stopped` set.
pub fn gmres(
    n: usize,
    k: usize,
    b: &[f64],
    x: &mut [f64],
    op: &(dyn Fn(&[f64], &mut [f64], usize) + Sync),
    prec: &(dyn Fn(&mut [f64], usize) + Sync),
    p: &GmresParams,
    on_iter: &(dyn Fn(usize, &[f64]) + Sync),
    stop: &(dyn Fn() -> bool + Sync),
) -> GmresStats {
    let mut st: Vec<State> = (0..k)
        .map(|r| State {
            bnorm: dot(&b[r * n..(r + 1) * n], &b[r * n..(r + 1) * n]).sqrt(),
            basis: vec![],
            h: vec![],
            cs: vec![],
            sn: vec![],
            g: vec![],
            size: 0,
            cycle_done: false,
            converged: false,
            iters: 0,
        })
        .collect();
    let mut restarts = 0;
    // relative residual estimates (|g_{j+1}| / |b|) for progress reporting
    let mut est = vec![1.0; k];
    let mut history: Vec<Vec<f64>> = Vec::new();
    let mut iteration = 0;
    let mut ax = vec![0.0; n * k];
    let mut stopped = false;
    let restart = p.restart.max(1); // 0 would never count an iteration
    let mut breakdown = false;
    'outer: loop {
        if stop() {
            stopped = true;
            break;
        }
        // residuals r = b − A x
        op(x, &mut ax, k);
        for (r, s) in st.iter_mut().enumerate() {
            if s.converged || s.iters >= p.max_iter {
                s.cycle_done = true;
                s.size = 0;
                continue;
            }
            let res: Vec<f64> = (0..n).map(|i| b[r * n + i] - ax[r * n + i]).collect();
            let beta = dot(&res, &res).sqrt();
            if !beta.is_finite() {
                // NaN/inf (e.g. a singular preconditioner block): no point iterating
                breakdown = true;
                s.cycle_done = true;
                continue;
            }
            if s.bnorm == 0.0 || beta <= p.tol * s.bnorm {
                s.converged = true;
                continue;
            }
            s.basis = vec![res.iter().map(|v| v / beta).collect()];
            s.h.clear();
            s.cs.clear();
            s.sn.clear();
            s.g = vec![beta];
            est[r] = beta / s.bnorm;
            s.size = 0;
            s.cycle_done = false;
        }
        if breakdown || st.iter().all(|s| s.converged || s.iters >= p.max_iter) {
            break;
        }
        for j in 0..restart {
            let active: Vec<usize> = (0..k).filter(|&r| !st[r].converged && !st[r].cycle_done).collect();
            if active.is_empty() {
                break;
            }
            let m = active.len();
            // W = A M⁻¹ v_j for every active right-hand side
            let mut z = vec![0.0; n * m];
            for (c, &r) in active.iter().enumerate() {
                z[c * n..(c + 1) * n].copy_from_slice(&st[r].basis[j]);
            }
            prec(&mut z, m);
            let mut w = vec![0.0; n * m];
            op(&z, &mut w, m);
            let wcols: Vec<Vec<f64>> = w.chunks_exact(n).map(|c| c.to_vec()).collect();
            let mut sel: Vec<(&mut State, Vec<f64>)> =
                st.iter_mut().enumerate().filter(|(i, _)| active.binary_search(i).is_ok()).map(|(_, s)| s).zip(wcols).collect();
            sel.par_iter_mut().for_each(|(s, w)| {
                // modified Gram–Schmidt, twice for robustness
                let mut hcol = vec![0.0; j + 2];
                for _ in 0..2 {
                    for i in 0..=j {
                        let hij = dot(w, &s.basis[i]);
                        hcol[i] += hij;
                        for (wv, bv) in w.iter_mut().zip(&s.basis[i]) {
                            *wv -= hij * bv;
                        }
                    }
                }
                let hn = dot(w, w).sqrt();
                hcol[j + 1] = hn;
                // apply previous rotations, then a new one that zeros hcol[j+1]
                for i in 0..j {
                    let (c, sv) = (s.cs[i], s.sn[i]);
                    let (a, bb) = (hcol[i], hcol[i + 1]);
                    hcol[i] = c * a + sv * bb;
                    hcol[i + 1] = -sv * a + c * bb;
                }
                let (a, bb) = (hcol[j], hcol[j + 1]);
                let rho = a.hypot(bb);
                let (c, sv) = if rho == 0.0 { (1.0, 0.0) } else { (a / rho, bb / rho) };
                hcol[j] = rho;
                hcol[j + 1] = 0.0;
                s.cs.push(c);
                s.sn.push(sv);
                let gj = s.g[j];
                s.g[j] = c * gj;
                s.g.push(-sv * gj);
                s.h.push(hcol);
                s.size = j + 1;
                s.iters += 1;
                if hn > 0.0 {
                    s.basis.push(w.iter().map(|v| v / hn).collect());
                }
                if s.g[j + 1].abs() <= p.tol * s.bnorm || hn == 0.0 || s.iters >= p.max_iter {
                    s.cycle_done = true;
                }
            });
            drop(sel);
            for &r in &active {
                let s = &st[r];
                est[r] = s.g[s.size].abs() / s.bnorm;
            }
            iteration += 1;
            if est.iter().any(|e| !e.is_finite()) {
                breakdown = true;
                break 'outer;
            }
            history.push(est.clone());
            on_iter(iteration, &est);
            if stop() {
                stopped = true;
                break 'outer;
            }
        }
        // x += M⁻¹ V y with H y = g (upper triangular)
        let upd: Vec<usize> = (0..k).filter(|&r| !st[r].converged && st[r].size > 0).collect();
        let m = upd.len();
        let mut corr = vec![0.0; n * m];
        for (c, &r) in upd.iter().enumerate() {
            let s = &st[r];
            let q = s.size;
            let mut y = vec![0.0; q];
            for i in (0..q).rev() {
                let mut acc = s.g[i];
                for l in (i + 1)..q {
                    acc -= s.h[l][i] * y[l];
                }
                y[i] = acc / s.h[i][i];
            }
            let out = &mut corr[c * n..(c + 1) * n];
            for (i, yi) in y.iter().enumerate() {
                for (o, v) in out.iter_mut().zip(&s.basis[i]) {
                    *o += yi * v;
                }
            }
        }
        prec(&mut corr, m);
        for (c, &r) in upd.iter().enumerate() {
            for i in 0..n {
                x[r * n + i] += corr[c * n + i];
            }
        }
        restarts += 1;
    }
    // true residuals (not needed when stopped: the result is discarded)
    if stopped {
        return GmresStats { iterations: st.iter().map(|s| s.iters).collect(), residual: vec![f64::NAN; k], converged: false, restarts, history, stopped };
    }
    op(x, &mut ax, k);
    let residual: Vec<f64> = (0..k)
        .map(|r| {
            let e: f64 = (0..n).map(|i| (b[r * n + i] - ax[r * n + i]).powi(2)).sum::<f64>().sqrt();
            if st[r].bnorm > 0.0 {
                e / st[r].bnorm
            } else {
                0.0
            }
        })
        .collect();
    let converged = !breakdown && residual.iter().all(|&e| e <= 10.0 * p.tol);
    GmresStats { iterations: st.iter().map(|s| s.iters).collect(), residual, converged, restarts, history, stopped }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solves_a_nonsymmetric_system() {
        // diagonally dominant non-symmetric test matrix
        let n = 200;
        let a = |i: usize, j: usize| if i == j { 4.0 + i as f64 * 0.01 } else { 1.0 / (1.0 + (i as f64 - j as f64 + 0.3).abs()) * 0.3 };
        let op = |x: &[f64], y: &mut [f64], m: usize| {
            for r in 0..m {
                for i in 0..n {
                    y[r * n + i] = (0..n).map(|j| a(i, j) * x[r * n + j]).sum();
                }
            }
        };
        let prec = |_: &mut [f64], _: usize| {};
        let k = 3;
        let b: Vec<f64> = (0..n * k).map(|i| ((i * 37 % 11) as f64) - 5.0).collect();
        let mut x = vec![0.0; n * k];
        let s = gmres(n, k, &b, &mut x, &op, &prec, &GmresParams { tol: 1e-12, restart: 3, max_iter: 500 }, &|_, _| {}, &|| false);
        assert!(!s.history.is_empty() && s.history.last().unwrap().iter().all(|&r| r < 1e-11));
        assert!(s.converged, "{s:?}");
        let mut ax = vec![0.0; n * k];
        op(&x, &mut ax, k);
        let err = ax.iter().zip(&b).map(|(p, q)| (p - q).abs()).fold(0.0, f64::max);
        assert!(err < 1e-9, "max residual {err}");
        assert!(s.restarts > 1, "restart path exercised");
    }
}
