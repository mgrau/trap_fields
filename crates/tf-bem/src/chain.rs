//! Equilibrium configuration and normal modes of N identical ions in the trap.
//!
//!   E(r_1..r_N) = Σ_i U(r_i) + Σ_{i<j} Q² / (4πε0 |r_i − r_j|)
//!
//! with U the single-ion trapping energy (pseudopotential + static, see [`Fields`]). The
//! equilibrium is found by damped Newton on all 3N coordinates, starting from a linear
//! chain along the weakest trap axis (slightly perturbed, so zigzag configurations are
//! reachable); the normal modes are the eigenvectors of the 3N×3N Hessian / m.

use crate::panel::{dot, norm, sub, V3};
use crate::trap::Fields;
use serde::Serialize;
use std::f64::consts::PI;

const EPS0: f64 = 8.854_187_8188e-12;

#[derive(Debug, Clone, Serialize)]
pub struct ChainMode {
    /// ω/2π (Hz).
    pub freq: f64,
    /// Normalized eigenvector: displacement of each ion (unitless, Σ|v_i|² = 1).
    pub vector: Vec<V3>,
    /// Fraction of the mode along each single-ion principal axis (sums to 1).
    pub axis_weights: [f64; 3],
}

#[derive(Debug, Clone, Serialize)]
pub struct ChainResult {
    pub n: usize,
    /// Equilibrium positions (m), sorted along the chain axis.
    pub positions: Vec<V3>,
    /// Direction the chain was started along (weakest single-ion mode).
    pub chain_axis: V3,
    /// Single-ion principal axes (unit vectors), ascending frequency.
    pub principal_axes: [V3; 3],
    /// Smallest ion–ion distance (m).
    pub min_spacing: f64,
    /// Largest displacement perpendicular to the chain axis, relative to the minimal spacing.
    pub transverse: f64,
    /// Modes in ascending frequency.
    pub modes: Vec<ChainMode>,
    /// Whether every Hessian eigenvalue is positive (a stable minimum).
    pub stable: bool,
    pub iterations: usize,
}

struct System<'a> {
    f: &'a Fields,
    n: usize,
    /// Coulomb prefactor Q²/(4πε0) (J·m).
    kc: f64,
}

impl System<'_> {
    fn energy(&self, x: &[V3]) -> f64 {
        let mut e: f64 = x.iter().map(|&r| self.f.energy(r).0).sum();
        for i in 0..self.n {
            for j in (i + 1)..self.n {
                e += self.kc / norm(sub(x[i], x[j]));
            }
        }
        e
    }

    fn gradient(&self, x: &[V3]) -> Vec<f64> {
        let mut g = vec![0.0; 3 * self.n];
        for i in 0..self.n {
            let gi = self.f.energy(x[i]).1;
            for k in 0..3 {
                g[3 * i + k] += gi[k];
            }
            for j in 0..self.n {
                if i != j {
                    let d = sub(x[i], x[j]);
                    let r = norm(d);
                    for k in 0..3 {
                        g[3 * i + k] -= self.kc * d[k] / (r * r * r);
                    }
                }
            }
        }
        g
    }

    fn hessian(&self, x: &[V3]) -> Vec<f64> {
        let m = 3 * self.n;
        let mut h = vec![0.0; m * m];
        for i in 0..self.n {
            let hi = self.f.hessian(x[i]);
            for a in 0..3 {
                for b in 0..3 {
                    h[(3 * i + a) * m + 3 * i + b] += hi[a][b];
                }
            }
            for j in 0..self.n {
                if i == j {
                    continue;
                }
                let d = sub(x[i], x[j]);
                let r = norm(d);
                let (r3, r5) = (r * r * r, r * r * r * r * r);
                for a in 0..3 {
                    for b in 0..3 {
                        let c = self.kc * (3.0 * d[a] * d[b] / r5 - if a == b { 1.0 / r3 } else { 0.0 });
                        h[(3 * i + a) * m + 3 * i + b] += c;
                        h[(3 * i + a) * m + 3 * j + b] -= c;
                    }
                }
            }
        }
        h
    }
}

/// Symmetric eigen-decomposition (ascending eigenvalues, eigenvectors as columns, m×m).
fn sym_eig(h: &[f64], m: usize) -> (Vec<f64>, Vec<f64>) {
    let a = faer::Mat::<f64>::from_fn(m, m, |i, j| 0.5 * (h[i * m + j] + h[j * m + i]));
    let e = a.self_adjoint_eigen(faer::Side::Lower).expect("eigendecomposition");
    let s = e.S();
    let u = e.U();
    let vals: Vec<f64> = (0..m).map(|i| s[i]).collect();
    let mut vecs = vec![0.0; m * m];
    for i in 0..m {
        for j in 0..m {
            vecs[i * m + j] = u[(i, j)];
        }
    }
    (vals, vecs)
}

/// Equilibrium and normal modes of `n` ions. Needs a confined single-ion trap.
pub fn solve_chain(f: &Fields, n: usize) -> Result<ChainResult, String> {
    if n == 0 || n > 200 {
        return Err("number of ions must be between 1 and 200".into());
    }
    let single = f.analyze();
    if !single.confined {
        return Err("the single-ion trap is not confined; no chain equilibrium".into());
    }
    let m_ion = f.ion.mass();
    let q = f.ion.charge();
    let kc = q * q / (4.0 * PI * EPS0);
    let axes = [single.modes[0].axis, single.modes[1].axis, single.modes[2].axis];
    let k_ax = single.modes[0].curvature;
    // characteristic length (kc / k)^(1/3) and the minimal spacing of a harmonic chain
    let ell = (kc / k_ax).cbrt();
    let spacing = 2.018 * ell / (n as f64).powf(0.559);
    let eq0 = single.equilibrium;
    let mut x: Vec<V3> = (0..n)
        .map(|i| {
            let s = (i as f64 - (n as f64 - 1.0) / 2.0) * spacing;
            // small deterministic transverse offsets break the symmetry (zigzag reachable)
            let t1 = 1e-3 * ell * ((i as f64 * 1.7).sin());
            let t2 = 1e-3 * ell * ((i as f64 * 2.3).cos());
            [0, 1, 2].map(|k| eq0[k] + s * axes[0][k] + t1 * axes[1][k] + t2 * axes[2][k])
        })
        .collect();
    let sys = System { f, n, kc };
    let dim = 3 * n;
    let mut e = sys.energy(&x);
    let mut mu = 0.0;
    let mut iterations = 0;
    for _ in 0..300 {
        iterations += 1;
        let g = sys.gradient(&x);
        let h = sys.hessian(&x);
        let (lam, v) = sym_eig(&h, dim);
        let lmax = lam.iter().fold(0.0f64, |a, l| a.max(l.abs())).max(1e-300);
        let floor = (-lam[0]).max(0.0) + 1e-12 * lmax;
        let mut accepted = false;
        let mut step_norm = 0.0;
        for _ in 0..40 {
            // δ = −(H + damp)⁻¹ g in the eigenbasis
            let damp = floor + mu;
            let mut step = vec![0.0; dim];
            for c in 0..dim {
                let gc: f64 = (0..dim).map(|r| v[r * dim + c] * g[r]).sum();
                let f = gc / (lam[c] + damp);
                for r in 0..dim {
                    step[r] -= f * v[r * dim + c];
                }
            }
            let sn = step.iter().map(|s| s * s).sum::<f64>().sqrt();
            let scale = if sn > 0.3 * spacing { 0.3 * spacing / sn } else { 1.0 };
            let trial: Vec<V3> = (0..n).map(|i| [0, 1, 2].map(|k| x[i][k] + scale * step[3 * i + k])).collect();
            let et = sys.energy(&trial);
            if et <= e || sn * scale < 1e-15 * ell {
                x = trial;
                e = et;
                mu *= 0.3;
                accepted = true;
                step_norm = sn * scale;
                break;
            }
            mu = if mu == 0.0 { 1e-6 * lmax } else { mu * 4.0 };
        }
        if !accepted || step_norm < 1e-13 * ell {
            break;
        }
    }
    // modes
    let h = sys.hessian(&x);
    let (lam, v) = sym_eig(&h, dim);
    let stable = lam[0] > 0.0;
    let modes: Vec<ChainMode> = (0..dim)
        .map(|c| {
            let vector: Vec<V3> = (0..n).map(|i| [v[(3 * i) * dim + c], v[(3 * i + 1) * dim + c], v[(3 * i + 2) * dim + c]]).collect();
            let mut w = [0.0; 3];
            for vi in &vector {
                for (a, ax) in axes.iter().enumerate() {
                    w[a] += dot(*vi, *ax).powi(2);
                }
            }
            let tot: f64 = w.iter().sum::<f64>().max(1e-300);
            ChainMode { freq: (lam[c].max(0.0) / m_ion).sqrt() / (2.0 * PI), vector, axis_weights: w.map(|x| x / tot) }
        })
        .collect();
    // sort ions along the chain axis
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| dot(sub(x[a], eq0), axes[0]).partial_cmp(&dot(sub(x[b], eq0), axes[0])).unwrap());
    let positions: Vec<V3> = order.iter().map(|&i| x[i]).collect();
    let modes: Vec<ChainMode> = modes
        .into_iter()
        .map(|md: ChainMode| ChainMode { vector: order.iter().map(|&i| md.vector[i]).collect(), ..md })
        .collect();
    let mut min_spacing = f64::INFINITY;
    for i in 0..n {
        for j in (i + 1)..n {
            min_spacing = min_spacing.min(norm(sub(x[i], x[j])));
        }
    }
    let transverse = if n > 1 {
        x.iter()
            .map(|&r| {
                let d = sub(r, eq0);
                let along = dot(d, axes[0]);
                norm(sub(d, axes[0].map(|a| a * along)))
            })
            .fold(0.0, f64::max)
            / min_spacing
    } else {
        0.0
    };
    Ok(ChainResult {
        n,
        positions,
        chain_axis: axes[0],
        principal_axes: axes,
        min_spacing: if n > 1 { min_spacing } else { 0.0 },
        transverse,
        modes,
        stable,
        iterations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harmonics::{idx, n_coeffs, Expansion};
    use crate::trap::Ion;

    /// Linear Paul trap along x with radial pseudopotential far stiffer than the axial DC.
    fn trap(u0: f64) -> Fields {
        let (v0, rr, d) = (300.0, 0.5e-3, 2e-3);
        let r0 = 1e-3;
        let s3 = 3f64.sqrt();
        let mut rf = vec![0.0; n_coeffs(4)];
        let k = v0 * r0 * r0 / (2.0 * rr * rr);
        rf[idx(2, 2)] = -k / s3;
        rf[idx(2, 0)] = -k;
        let mut dc = vec![0.0; n_coeffs(4)];
        let kd = u0 * r0 * r0 / (d * d);
        dc[idx(2, 2)] = kd * s3 / 2.0;
        dc[idx(2, 0)] = -kd / 2.0;
        Fields {
            rf: Expansion { lmax: 4, center: [0.0; 3], r0, coeffs: rf },
            dc: Expansion { lmax: 4, center: [0.0; 3], r0, coeffs: dc },
            omega: 2.0 * PI * 30e6,
            ion: Ion { mass_amu: 40.0, charge_e: 1.0 },
            r_valid: 1e-3,
        }
    }

    #[test]
    fn two_and_three_ions_match_harmonic_results() {
        let f = trap(2.0);
        let one = f.analyze();
        let (wz, wr) = (one.modes[0].freq_pseudo.unwrap(), one.modes[1].freq_pseudo.unwrap());
        let m = f.ion.mass();
        let q = f.ion.charge();
        let kc = q * q / (4.0 * PI * EPS0);
        let kz = m * (2.0 * PI * wz).powi(2);
        let ell = (kc / kz).cbrt();

        // two ions: separation (2 kc / kz)^(1/3); modes COM ω_z, stretch √3 ω_z,
        // radial COM ω_r and rocking sqrt(ω_r² − ω_z²)
        let c2 = solve_chain(&f, 2).unwrap();
        assert!(c2.stable && c2.transverse < 1e-6);
        let d2 = (2.0f64).cbrt() * ell;
        assert!((c2.min_spacing / d2 - 1.0).abs() < 1e-6, "{} vs {d2}", c2.min_spacing);
        let fr: Vec<f64> = c2.modes.iter().map(|m| m.freq).collect();
        let expect = [wz, 3f64.sqrt() * wz, (wr * wr - wz * wz).sqrt(), (wr * wr - wz * wz).sqrt(), wr, wr];
        let mut e = expect.to_vec();
        e.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for (a, b) in fr.iter().zip(&e) {
            assert!((a / b - 1.0).abs() < 1e-5, "modes {fr:?} vs {e:?}");
        }
        assert!(c2.modes[0].axis_weights[0] > 0.999, "lowest mode is axial COM");

        // three ions: positions 0, ±(5/4)^(1/3) ℓ; axial modes ω, √3 ω, √(29/5) ω
        let c3 = solve_chain(&f, 3).unwrap();
        let x: Vec<f64> = c3.positions.iter().map(|p| p[0]).collect();
        let a = (1.25f64).cbrt() * ell;
        assert!((x[0] + a).abs() < 1e-6 * a && x[1].abs() < 1e-6 * a && (x[2] - a).abs() < 1e-6 * a, "{x:?}");
        let axial: Vec<f64> = c3.modes.iter().filter(|m| m.axis_weights[0] > 0.99).map(|m| m.freq / wz).collect();
        let want = [1.0, 3f64.sqrt(), (29.0f64 / 5.0).sqrt()];
        assert_eq!(axial.len(), 3);
        for (a, b) in axial.iter().zip(want) {
            assert!((a - b).abs() < 1e-5, "axial {axial:?}");
        }
    }

    #[test]
    fn strong_axial_confinement_gives_a_zigzag() {
        // raise the axial voltage until ω_z approaches ω_r: a long chain buckles
        let f = trap(60.0);
        let one = f.analyze();
        let (wz, wr) = (one.modes[0].freq_pseudo.unwrap(), one.modes[1].freq_pseudo.unwrap());
        assert!(wr / wz < 2.0, "need a weak radial/axial ratio: {}", wr / wz);
        let c = solve_chain(&f, 10).unwrap();
        assert!(c.stable, "minimum found");
        assert!(c.transverse > 0.1, "zigzag expected, transverse {}", c.transverse);
    }
}
