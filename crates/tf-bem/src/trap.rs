//! Ion-trap analysis from the potential expansions: pseudopotential, equilibrium,
//! secular modes, Mathieu parameters, exact linear (Floquet) stability, anharmonicity,
//! and a trap-depth estimate from the surface charges.
//!
//! The RF electrodes carry V_e cos Ωt (amplitude V_e, zero to peak), the DC electrodes
//! static voltages. With φ_rf the RF *amplitude* potential and φ_dc the static one,
//!
//!   U(r) = Q² |∇φ_rf|² / (4 m Ω²) + Q φ_dc(r)
//!
//! Mathieu form along a principal axis u (τ = Ωt/2):  ẍ + (a − 2q cos 2τ) x = 0 with
//! a = 4Q uᵀH_dc u /(mΩ²) and q = −2Q uᵀH_rf u /(mΩ²) (H: potential Hessians).

use crate::harmonics::Expansion;
use crate::panel::{dot, norm, ray_hit, sub, Panel, V3};
use crate::solver::evaluate;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

pub const AMU: f64 = 1.660_539_068_92e-27;
pub const E_CHARGE: f64 = 1.602_176_634e-19;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Ion {
    pub mass_amu: f64,
    pub charge_e: f64,
}

impl Ion {
    pub fn mass(&self) -> f64 {
        self.mass_amu * AMU
    }
    pub fn charge(&self) -> f64 {
        self.charge_e * E_CHARGE
    }
}

/// Fields near the trap center from expansions (valid within `r_valid` of their center).
pub struct Fields {
    /// RF amplitude potential (V).
    pub rf: Expansion,
    /// Static potential (V).
    pub dc: Expansion,
    /// RF angular frequency Ω (rad/s).
    pub omega: f64,
    pub ion: Ion,
    pub r_valid: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Mode {
    /// Principal axis (unit vector).
    pub axis: V3,
    /// Total curvature k = ∂²U/∂s² along the axis (J/m²).
    pub curvature: f64,
    /// Pseudopotential contribution to the curvature (J/m²).
    pub curvature_rf: f64,
    /// sqrt(k/m)/2π (Hz); None if k ≤ 0 (not confined).
    pub freq_pseudo: Option<f64>,
    /// From the Floquet multipliers of the linearized equations (Hz).
    pub freq_floquet: Option<f64>,
    /// Polynomial fit along the axis: U(s) − U_eq ≈ c2 s² + c3 s³ + c4 s⁴ (J/m^n).
    pub c2: f64,
    pub c3: f64,
    pub c4: f64,
}

/// Mathieu parameters along a principal axis of the RF quadrupole (the RF potential's
/// Hessian), where the RF force separates: q = −2Q λ_rf/(mΩ²), a = 4Q uᵀH_dc u/(mΩ²).
#[derive(Debug, Clone, Serialize)]
pub struct MathieuAxis {
    pub axis: V3,
    pub a: f64,
    pub q: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Analysis {
    /// Point of minimum |∇φ_rf| (m).
    pub rf_null: V3,
    /// |∇φ_rf| there (V/m).
    pub rf_null_residual: f64,
    /// Direction of the RF null line, if the RF field vanishes along a line.
    pub rf_null_line: Option<V3>,
    pub equilibrium: V3,
    /// Whether a minimum of U was found inside the region where the expansions are valid.
    pub confined: bool,
    /// RF field amplitude at the equilibrium (V/m): drives excess micromotion.
    pub rf_field_at_eq: V3,
    /// U at the equilibrium (J).
    pub energy_eq: f64,
    pub modes: Vec<Mode>,
    /// Sorted by |q|, largest first.
    pub mathieu: Vec<MathieuAxis>,
    /// All Floquet multipliers on the unit circle.
    pub stable: Option<bool>,
    pub notes: Vec<String>,
}

impl Fields {
    fn m(&self) -> f64 {
        self.ion.mass()
    }
    fn q(&self) -> f64 {
        self.ion.charge()
    }
    /// Pseudopotential prefactor Q²/(4mΩ²) (J per (V/m)²).
    pub fn c_pp(&self) -> f64 {
        self.q().powi(2) / (4.0 * self.m() * self.omega.powi(2))
    }

    /// (U, ∇U, U_pseudo) at r.
    pub fn energy(&self, r: V3) -> (f64, V3, f64) {
        let rf = self.rf.derivatives(r);
        let dc = self.dc.derivatives(r);
        let c = self.c_pp();
        let q = self.q();
        let g = rf.grad;
        let upp = c * dot(g, g);
        let mut grad = [0.0; 3];
        for i in 0..3 {
            grad[i] = 2.0 * c * dot(rf.hessian[i], g) + q * dc.grad[i];
        }
        (upp + q * dc.phi, grad, upp)
    }

    /// Hessian of the pseudopotential part (J/m²), by central differences of its analytic gradient.
    fn hessian_pp(&self, r: V3) -> [[f64; 3]; 3] {
        let c = self.c_pp();
        let grad_pp = |p: V3| {
            let d = self.rf.derivatives(p);
            [0, 1, 2].map(|i| 2.0 * c * dot(d.hessian[i], d.grad))
        };
        let h = 1e-4 * self.rf.r0;
        let mut k = [[0.0; 3]; 3];
        for j in 0..3 {
            let (mut a, mut b) = (r, r);
            a[j] += h;
            b[j] -= h;
            let (ga, gb) = (grad_pp(a), grad_pp(b));
            for i in 0..3 {
                k[i][j] = (ga[i] - gb[i]) / (2.0 * h);
            }
        }
        symmetrize(k)
    }

    fn hessian_dc(&self, r: V3) -> [[f64; 3]; 3] {
        let q = self.q();
        self.dc.derivatives(r).hessian.map(|row| row.map(|x| q * x))
    }

    pub fn hessian(&self, r: V3) -> [[f64; 3]; 3] {
        add3(self.hessian_pp(r), self.hessian_dc(r))
    }

    fn inside(&self, r: V3) -> bool {
        norm(sub(r, self.rf.center)) < 0.95 * self.r_valid
    }

    /// Least-squares zero of ∇φ_rf (Levenberg–Marquardt; minimum-norm on a null line).
    fn rf_null(&self) -> (V3, f64, Option<V3>) {
        let mut x = self.rf.center;
        for _ in 0..60 {
            let d = self.rf.derivatives(x);
            let (lam, vec) = sym_eig3(d.hessian);
            let lmax = lam.iter().fold(0.0f64, |m, l| m.max(l.abs()));
            if lmax == 0.0 {
                break;
            }
            // δ = −(H² + μ)⁻¹ H g in the eigenbasis
            let mut step = [0.0; 3];
            for k in 0..3 {
                let v = [vec[0][k], vec[1][k], vec[2][k]];
                let gk = dot(v, d.grad);
                let f = lam[k] / (lam[k] * lam[k] + 1e-12 * lmax * lmax);
                for i in 0..3 {
                    step[i] -= f * gk * v[i];
                }
            }
            let next = [x[0] + step[0], x[1] + step[1], x[2] + step[2]];
            if !self.inside(next) {
                break;
            }
            x = next;
            if norm(step) < 1e-13 * self.rf.r0 {
                break;
            }
        }
        let d = self.rf.derivatives(x);
        let (lam, vec) = sym_eig3(d.hessian);
        let lmax = lam.iter().fold(0.0f64, |m, l| m.max(l.abs()));
        // a near-zero eigenvalue of H_rf means the null extends along that direction
        let line = (0..3).find(|&k| lam[k].abs() < 1e-4 * lmax).map(|k| canonical([vec[0][k], vec[1][k], vec[2][k]]));
        (x, norm(d.grad), line)
    }

    /// Damped Newton descent on U from `start`; returns (point, converged inside the valid region).
    fn minimize(&self, start: V3) -> (V3, bool) {
        let mut x = start;
        let (mut u, mut g, _) = self.energy(x);
        let mut mu = 0.0;
        for _ in 0..200 {
            let k = self.hessian(x);
            let (lam, vec) = sym_eig3(k);
            let kmax = lam.iter().fold(0.0f64, |m, l| m.max(l.abs())).max(1e-300);
            let floor = (-lam[0]).max(0.0) + 1e-9 * kmax;
            let mut accepted = false;
            for _ in 0..30 {
                let damp = floor + mu;
                let mut step = [0.0; 3];
                for j in 0..3 {
                    let v = [vec[0][j], vec[1][j], vec[2][j]];
                    let f = dot(v, g) / (lam[j] + damp);
                    for i in 0..3 {
                        step[i] -= f * v[i];
                    }
                }
                // never step further than a tenth of the valid radius at once
                let sn = norm(step);
                if sn > 0.1 * self.r_valid {
                    step = step.map(|s| s * 0.1 * self.r_valid / sn);
                }
                let next = [x[0] + step[0], x[1] + step[1], x[2] + step[2]];
                let (un, gn, _) = self.energy(next);
                if un <= u || sn < 1e-14 * self.rf.r0 {
                    if !self.inside(next) {
                        return (next, false);
                    }
                    let done = norm(step) < 1e-12 * self.rf.r0;
                    x = next;
                    u = un;
                    g = gn;
                    mu *= 0.3;
                    accepted = true;
                    if done {
                        let (lam, _) = sym_eig3(self.hessian(x));
                        return (x, lam[0] > 0.0);
                    }
                    break;
                }
                mu = if mu == 0.0 { 1e-6 * kmax } else { mu * 4.0 };
            }
            if !accepted {
                break;
            }
        }
        let (lam, _) = sym_eig3(self.hessian(x));
        (x, lam[0] > 0.0 && self.inside(x))
    }

    pub fn analyze(&self) -> Analysis {
        let mut notes = Vec::new();
        let (rf_null, rf_null_residual, rf_null_line) = self.rf_null();
        let (eq, mut confined) = self.minimize(rf_null);
        if !confined {
            notes.push("No minimum of the trapping potential inside the expansion region: the ion is not confined in every direction.".into());
        }
        let k_pp = self.hessian_pp(eq);
        let k_dc = self.hessian_dc(eq);
        let k = add3(k_pp, k_dc);
        let (lam, vec) = sym_eig3(k);
        let rf = self.rf.derivatives(eq);
        let dc = self.dc.derivatives(eq);
        let (m, q, w) = (self.m(), self.q(), self.omega);
        let (energy_eq, _, _) = self.energy(eq);

        let floquet = floquet(&rf.hessian, &dc.hessian, q / (m * w * w));
        let stable = floquet.as_ref().map(|f| f.0);
        let mut ffreqs: Vec<f64> = floquet.map(|f| f.1).unwrap_or_default().iter().map(|b| b * w / (2.0 * PI)).collect();
        ffreqs.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let (lrf, vrf) = sym_eig3(rf.hessian);
        let mut mathieu: Vec<MathieuAxis> = (0..3)
            .map(|j| {
                let axis = canonical([vrf[0][j], vrf[1][j], vrf[2][j]]);
                let hd = dot(axis, [dot(dc.hessian[0], axis), dot(dc.hessian[1], axis), dot(dc.hessian[2], axis)]);
                MathieuAxis { axis, a: 4.0 * q * hd / (m * w * w), q: -2.0 * q * lrf[j] / (m * w * w) }
            })
            .collect();
        mathieu.sort_by(|x, y| y.q.abs().partial_cmp(&x.q.abs()).unwrap());
        let mut modes = Vec::with_capacity(3);
        for j in 0..3 {
            let axis = canonical([vec[0][j], vec[1][j], vec[2][j]]);
            let along = |h: &[[f64; 3]; 3]| dot(axis, [dot(h[0], axis), dot(h[1], axis), dot(h[2], axis)]);
            let kk = lam[j];
            let (c2, c3, c4) = self.anharmonic(eq, axis);
            modes.push(Mode {
                axis,
                curvature: kk,
                curvature_rf: along(&k_pp),
                freq_pseudo: (kk > 0.0).then(|| (kk / m).sqrt() / (2.0 * PI)),
                freq_floquet: None,
                c2,
                c3,
                c4,
            });
        }
        // Floquet frequencies are not tied to axes; match them to the modes by order
        if stable == Some(true) && ffreqs.len() == 3 && modes.iter().all(|m| m.freq_pseudo.is_some()) {
            for (md, f) in modes.iter_mut().zip(&ffreqs) {
                md.freq_floquet = Some(*f);
            }
        }
        if stable == Some(false) {
            notes.push("Linearized motion is unstable (Floquet multipliers off the unit circle): outside the Mathieu stability region.".into());
            confined = false;
        }
        let offset = norm(sub(eq, rf_null));
        if confined && offset > 1e-3 * self.r_valid {
            notes.push(format!(
                "The equilibrium is {:.2} µm from the RF null (RF field {:.3e} V/m there): excess micromotion. The static fields \
                 overcome the pseudopotential at the null; the Floquet frequencies linearize about a point with nonzero RF field \
                 and can differ from the pseudopotential ones.",
                offset * 1e6,
                norm(rf.grad)
            ));
        }
        if mathieu[0].q.abs() > 0.4 {
            notes.push(format!("|q| = {:.2}: the pseudopotential approximation becomes inaccurate; compare with the Floquet frequencies.", mathieu[0].q.abs()));
        }
        Analysis {
            rf_null,
            rf_null_residual,
            rf_null_line,
            equilibrium: eq,
            confined,
            rf_field_at_eq: rf.grad.map(|g| -g),
            energy_eq,
            modes,
            mathieu,
            stable,
            notes,
        }
    }

    /// Least-squares polynomial (degree 6) of U along `axis` through `eq`; returns c2, c3, c4.
    fn anharmonic(&self, eq: V3, axis: V3) -> (f64, f64, f64) {
        let room = self.r_valid - norm(sub(eq, self.rf.center));
        let smax = 0.3 * room.max(0.0);
        if smax <= 0.0 {
            return (0.0, 0.0, 0.0);
        }
        let u0 = self.energy(eq).0;
        let n = 41;
        // basis t^1..t^6, t = s/smax
        let mut ata = [[0.0; 6]; 6];
        let mut atb = [0.0; 6];
        for i in 0..n {
            let t = -1.0 + 2.0 * i as f64 / (n - 1) as f64;
            let s = t * smax;
            let u = self.energy([eq[0] + s * axis[0], eq[1] + s * axis[1], eq[2] + s * axis[2]]).0 - u0;
            let phi: [f64; 6] = std::array::from_fn(|k| t.powi(k as i32 + 1));
            for a in 0..6 {
                atb[a] += phi[a] * u;
                for b in 0..6 {
                    ata[a][b] += phi[a] * phi[b];
                }
            }
        }
        let c = solve_dense(ata, atb);
        (c[1] / smax.powi(2), c[2] / smax.powi(3), c[3] / smax.powi(4))
    }
}

/// Floquet analysis of ẍ = −κ (H_dc + H_rf cos τ) x over one RF period (τ = Ωt):
/// (stable, the three β = ω/Ω... as fractions ω/Ω of the RF frequency).
fn floquet(h_rf: &[[f64; 3]; 3], h_dc: &[[f64; 3]; 3], kappa: f64) -> Option<(bool, Vec<f64>)> {
    let n = 4000;
    let dt = 2.0 * PI / n as f64;
    // state: 6×6 matrix Y (columns), dY/dτ = A(τ) Y with A = [[0, I], [−κ H(τ), 0]]
    let deriv = |tau: f64, y: &[[f64; 6]; 6]| -> [[f64; 6]; 6] {
        let c = tau.cos();
        let mut out = [[0.0; 6]; 6];
        for col in 0..6 {
            for i in 0..3 {
                out[i][col] = y[i + 3][col];
                let mut acc = 0.0;
                for j in 0..3 {
                    acc += (h_dc[i][j] + h_rf[i][j] * c) * y[j][col];
                }
                out[i + 3][col] = -kappa * acc;
            }
        }
        out
    };
    let axpy = |y: &[[f64; 6]; 6], k: &[[f64; 6]; 6], s: f64| -> [[f64; 6]; 6] { std::array::from_fn(|i| std::array::from_fn(|j| y[i][j] + s * k[i][j])) };
    let mut y: [[f64; 6]; 6] = std::array::from_fn(|i| std::array::from_fn(|j| (i == j) as u8 as f64));
    for step in 0..n {
        let t = step as f64 * dt;
        let k1 = deriv(t, &y);
        let k2 = deriv(t + dt / 2.0, &axpy(&y, &k1, dt / 2.0));
        let k3 = deriv(t + dt / 2.0, &axpy(&y, &k2, dt / 2.0));
        let k4 = deriv(t + dt, &axpy(&y, &k3, dt));
        y = std::array::from_fn(|i| std::array::from_fn(|j| y[i][j] + dt / 6.0 * (k1[i][j] + 2.0 * k2[i][j] + 2.0 * k3[i][j] + k4[i][j])));
    }
    let m = faer::Mat::<f64>::from_fn(6, 6, |i, j| y[i][j]);
    let ev = m.eigenvalues().ok()?;
    let stable = ev.iter().all(|z| (z.re.hypot(z.im) - 1.0).abs() < 1e-6);
    // multipliers e^{±i 2π β}: keep one of each conjugate pair
    let mut betas: Vec<f64> = ev.iter().filter(|z| z.im > 0.0 || (z.im == 0.0 && z.re < 0.0)).map(|z| z.im.atan2(z.re).abs() / (2.0 * PI)).collect();
    betas.truncate(3);
    Some((stable, betas))
}

/// U (J) at points by direct integration over the surface charges (valid everywhere
/// outside the electrodes). `u_rf`, `u_dc` are σ/ε0 for the RF amplitudes and DC voltages.
pub fn energy_direct(panels: &[Panel], u_rf: &[f64], u_dc: &[f64], ion: Ion, omega: f64, points: &[V3]) -> Vec<f64> {
    let rf = evaluate(panels, u_rf, points);
    let dc = evaluate(panels, u_dc, points);
    let c = ion.charge().powi(2) / (4.0 * ion.mass() * omega * omega);
    rf.iter().zip(&dc).map(|((_, g), (phi, _))| c * dot(*g, *g) + ion.charge() * phi).collect()
}

/// Whether p lies inside a closed surface (ray-crossing parity).
pub fn inside_surface(panels: &[Panel], p: V3) -> bool {
    const D: V3 = [0.577_215_664_9, 0.618_033_988_7, 0.533_570_2];
    let n = norm(D);
    let d = D.map(|x| x / n);
    panels.iter().filter(|q| ray_hit(q, p, d).is_some()).count() % 2 == 1
}

#[derive(Debug, Clone, Serialize)]
pub struct Depth {
    /// Lowest barrier over all rays minus U_eq (J).
    pub depth: f64,
    /// Direction of the lowest barrier from the equilibrium.
    pub escape_direction: V3,
    /// Location of that barrier maximum (m).
    pub barrier_point: V3,
    pub n_rays: usize,
}

/// Trap depth estimate: along straight rays from the equilibrium (to the first electrode
/// hit, or `r_far`), take the highest U; the depth is the lowest of these barriers. The
/// true saddle can be lower, since escape paths need not be straight.
pub fn depth_estimate(panels: &[Panel], u_rf: &[f64], u_dc: &[f64], ion: Ion, omega: f64, eq: V3, u_eq: f64, r_far: f64) -> Depth {
    let n_rays = 400;
    let n_s = 48;
    let golden = PI * (3.0 - 5f64.sqrt());
    let dirs: Vec<V3> = (0..n_rays)
        .map(|i| {
            let z = 1.0 - 2.0 * (i as f64 + 0.5) / n_rays as f64;
            let r = (1.0 - z * z).sqrt();
            let phi = golden * i as f64;
            [r * phi.cos(), r * phi.sin(), z]
        })
        .collect();
    let ends: Vec<f64> = dirs
        .par_iter()
        .map(|&d| panels.iter().filter_map(|p| ray_hit(p, eq, d)).fold(r_far, f64::min))
        .collect();
    let mut pts = Vec::with_capacity(n_rays * n_s);
    for (d, &t_end) in dirs.iter().zip(&ends) {
        for k in 1..=n_s {
            // denser near the equilibrium; stop just short of the electrode surface
            let s = 0.995 * t_end * (k as f64 / n_s as f64).powf(1.5);
            pts.push([eq[0] + s * d[0], eq[1] + s * d[1], eq[2] + s * d[2]]);
        }
    }
    let u = energy_direct(panels, u_rf, u_dc, ion, omega, &pts);
    let mut best = (f64::INFINITY, 0usize, 0usize);
    for r in 0..n_rays {
        let (k, &umax) = u[r * n_s..(r + 1) * n_s].iter().enumerate().fold((0, &f64::NEG_INFINITY), |a, b| if b.1 > a.1 { b } else { a });
        if umax < best.0 {
            best = (umax, r, k);
        }
    }
    Depth { depth: best.0 - u_eq, escape_direction: dirs[best.1], barrier_point: pts[best.1 * n_s + best.2], n_rays }
}

// ------------------------------------------------------------------ small linear algebra

fn add3(a: [[f64; 3]; 3], b: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    std::array::from_fn(|i| std::array::from_fn(|j| a[i][j] + b[i][j]))
}

fn symmetrize(a: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    std::array::from_fn(|i| std::array::from_fn(|j| 0.5 * (a[i][j] + a[j][i])))
}

/// Unit vector with its largest component positive.
fn canonical(v: V3) -> V3 {
    let n = norm(v);
    let i = (0..3).max_by(|&a, &b| v[a].abs().partial_cmp(&v[b].abs()).unwrap()).unwrap();
    let s = if v[i] < 0.0 { -1.0 / n } else { 1.0 / n };
    v.map(|x| x * s)
}

/// Eigen-decomposition of a symmetric 3×3 matrix (cyclic Jacobi): ascending eigenvalues,
/// eigenvectors as the columns of the returned matrix.
pub fn sym_eig3(a: [[f64; 3]; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
    let mut a = symmetrize(a);
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..50 {
        let off = a[0][1].powi(2) + a[0][2].powi(2) + a[1][2].powi(2);
        let scale = a.iter().flatten().map(|x| x * x).sum::<f64>();
        if off <= 1e-30 * scale || off == 0.0 {
            break;
        }
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if a[p][q] == 0.0 {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
            let t = if theta == 0.0 { 1.0 } else { t };
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            for k in 0..3 {
                let (akp, akq) = (a[k][p], a[k][q]);
                a[k][p] = c * akp - s * akq;
                a[k][q] = s * akp + c * akq;
            }
            for k in 0..3 {
                let (apk, aqk) = (a[p][k], a[q][k]);
                a[p][k] = c * apk - s * aqk;
                a[q][k] = s * apk + c * aqk;
            }
            for k in 0..3 {
                let (vkp, vkq) = (v[k][p], v[k][q]);
                v[k][p] = c * vkp - s * vkq;
                v[k][q] = s * vkp + c * vkq;
            }
        }
    }
    let mut order = [0, 1, 2];
    order.sort_by(|&i, &j| a[i][i].partial_cmp(&a[j][j]).unwrap());
    let lam = order.map(|i| a[i][i]);
    let vec = std::array::from_fn(|r| order.map(|i| v[r][i]));
    (lam, vec)
}

/// Gaussian elimination with partial pivoting for a small dense system.
fn solve_dense<const N: usize>(mut a: [[f64; N]; N], mut b: [f64; N]) -> [f64; N] {
    for c in 0..N {
        let p = (c..N).max_by(|&i, &j| a[i][c].abs().partial_cmp(&a[j][c].abs()).unwrap()).unwrap();
        a.swap(c, p);
        b.swap(c, p);
        if a[c][c] == 0.0 {
            continue;
        }
        for r in (c + 1)..N {
            let f = a[r][c] / a[c][c];
            for k in c..N {
                a[r][k] -= f * a[c][k];
            }
            b[r] -= f * b[c];
        }
    }
    let mut x = [0.0; N];
    for r in (0..N).rev() {
        let s: f64 = ((r + 1)..N).map(|k| a[r][k] * x[k]).sum();
        x[r] = if a[r][r] == 0.0 { 0.0 } else { (b[r] - s) / a[r][r] };
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harmonics::{idx, n_coeffs};

    /// Ideal linear Paul trap, axis x: φ_rf = V0 (y² − z²)/(2R²), φ_dc = U0 (x² − (y²+z²)/2)/d²
    /// plus an optional uniform stray field E along y.
    fn ideal(v0: f64, rr: f64, u0: f64, d: f64, e_stray: f64) -> Fields {
        let r0 = 1e-3;
        let s3 = 3f64.sqrt();
        let mut rf = vec![0.0; n_coeffs(4)];
        // y² − z² = −Y22/√3 − Y20
        let k = v0 * r0 * r0 / (2.0 * rr * rr);
        rf[idx(2, 2)] = -k / s3;
        rf[idx(2, 0)] = -k;
        let mut dc = vec![0.0; n_coeffs(4)];
        // x² − (y²+z²)/2 = (√3/2) Y22 − Y20/2
        let kd = u0 * r0 * r0 / (d * d);
        dc[idx(2, 2)] = kd * s3 / 2.0;
        dc[idx(2, 0)] = -kd / 2.0;
        dc[idx(1, -1)] = -e_stray * r0; // φ = −E y
        Fields {
            rf: Expansion { lmax: 4, center: [0.0; 3], r0, coeffs: rf },
            dc: Expansion { lmax: 4, center: [0.0; 3], r0, coeffs: dc },
            omega: 2.0 * PI * 30e6,
            ion: Ion { mass_amu: 40.0, charge_e: 1.0 },
            r_valid: 1e-3,
        }
    }

    #[test]
    fn ideal_linear_trap() {
        let (v0, rr, u0, d) = (100.0, 0.5e-3, 5.0, 2e-3);
        let f = ideal(v0, rr, u0, d, 0.0);
        let a = f.analyze();
        let (m, q, w) = (f.ion.mass(), f.ion.charge(), f.omega);
        let wp2 = (q * v0 / (rr * rr)).powi(2) / (2.0 * m * m * w * w);
        let wz2 = 2.0 * q * u0 / (m * d * d);
        let wr2 = wp2 - q * u0 / (m * d * d);
        assert!(a.confined && a.stable == Some(true), "{a:?}");
        assert!(norm(a.equilibrium) < 1e-12 && a.rf_null_residual < 1e-9);
        let line = a.rf_null_line.expect("linear trap has an RF null line");
        assert!((line[0].abs() - 1.0).abs() < 1e-9);
        let fz = wz2.sqrt() / (2.0 * PI);
        let fr = wr2.sqrt() / (2.0 * PI);
        let m0 = &a.modes[0];
        assert!((m0.axis[0] - 1.0).abs() < 1e-9, "axial mode along x");
        assert!((m0.freq_pseudo.unwrap() / fz - 1.0).abs() < 1e-6, "{} vs {fz}", m0.freq_pseudo.unwrap());
        // RF principal axes y, z: |q| = 2QV0/(mΩ²R²), a = 4Q·(−U0/d²)/(mΩ²); axial (x): q = 0
        for mx in &a.mathieu[..2] {
            assert!(mx.axis[0].abs() < 1e-9);
            assert!((mx.q.abs() - 2.0 * q * v0 / (m * w * w * rr * rr)).abs() < 1e-9);
            assert!((mx.a + 4.0 * q * u0 / (m * w * w * d * d)).abs() < 1e-9);
        }
        assert!(a.mathieu[2].q.abs() < 1e-12 && (a.mathieu[2].a - 8.0 * q * u0 / (m * w * w * d * d)).abs() < 1e-9);
        for md in &a.modes[1..] {
            assert!((md.freq_pseudo.unwrap() / fr - 1.0).abs() < 1e-6, "{} vs {fr}", md.freq_pseudo.unwrap());
            // purely quadratic potential: no anharmonicity
            assert!(md.c3.abs() < 1e-6 * md.c2 / 1e-6 && md.c4.abs() < 1e-6 * md.c2 / 1e-12);
        }
        // axial: no RF, so Floquet = sqrt(a)Ω/2 exactly; radial: β ≈ sqrt(a + q²/2) at small q
        let ff = m0.freq_floquet.unwrap();
        assert!((ff / fz - 1.0).abs() < 1e-8, "axial Floquet {ff} vs {fz}");
        for md in &a.modes[1..] {
            let fl = md.freq_floquet.unwrap();
            assert!((fl / md.freq_pseudo.unwrap() - 1.0).abs() < 0.02, "radial Floquet {fl} vs pseudo {}", md.freq_pseudo.unwrap());
        }
        println!("q = {:.4}, f_axial {:.4} MHz, f_radial pseudo {:.4} / Floquet {:.4} MHz", a.mathieu[0].q, fz / 1e6, fr / 1e6, a.modes[1].freq_floquet.unwrap() / 1e6);
    }

    #[test]
    fn stray_field_displaces_the_ion() {
        let (v0, rr, u0, d, e) = (100.0, 0.5e-3, 5.0, 2e-3, 20.0);
        let f = ideal(v0, rr, u0, d, e);
        let a = f.analyze();
        let ky = a.modes[1].curvature; // radial curvature (both radial modes are degenerate here)
        let expect = f.ion.charge() * e / ky;
        assert!((a.equilibrium[1] - expect).abs() < 1e-6 * expect, "{} vs {expect}", a.equilibrium[1]);
        // RF field at the displaced ion: |E_rf| = V0 y / R²
        let erf = norm(a.rf_field_at_eq);
        assert!((erf - v0 * expect / (rr * rr)).abs() < 1e-6 * erf);
    }

    #[test]
    fn unconfined_without_rf() {
        let f = ideal(0.0, 0.5e-3, 5.0, 2e-3, 0.0);
        let a = f.analyze();
        assert!(!a.confined);
        assert!(a.modes[0].freq_pseudo.is_none());
    }

    #[test]
    fn jacobi_eigen() {
        let m = [[4.0, 1.0, -2.0], [1.0, 2.0, 0.5], [-2.0, 0.5, 3.0]];
        let (l, v) = sym_eig3(m);
        for k in 0..3 {
            let x = [v[0][k], v[1][k], v[2][k]];
            for i in 0..3 {
                assert!((dot(m[i], x) - l[k] * x[i]).abs() < 1e-12);
            }
        }
        assert!(l[0] <= l[1] && l[1] <= l[2]);
    }
}
