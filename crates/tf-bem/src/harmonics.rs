//! Real regular solid harmonics in Racah normalization, and potential expansions
//!
//!   φ(r) = Σ_{l ≤ L} Σ_{m=-l..l} a_lm · Y_lm((r − c)/r0)
//!
//! with Y_l0 = rˡ P_l(cos θ), Y_lm = N_lm rˡ P_l^m(cos θ) cos mφ (m > 0) and the same with
//! sin |m|φ for m < 0, N_lm = sqrt(2 (l−m)!/(l+m)!), no Condon–Shortley phase. Low orders:
//!
//!   l=1:  Y_1,1 = x   Y_1,-1 = y   Y_1,0 = z
//!   l=2:  Y_2,0 = z² − (x²+y²)/2   Y_2,1 = √3 xz   Y_2,-1 = √3 yz
//!         Y_2,2 = (√3/2)(x² − y²)   Y_2,-2 = √3 xy
//!
//! With this normalization  1/|r − r'| = Σ_lm Y_lm(r) Y_lm(r') / |r'|^{2l+1}  for |r| < |r'|,
//! so the coefficients follow directly from the BEM surface charges (no fitting).

use crate::panel::{dot, sub, Panel, V3, RADON7};
use serde::{Deserialize, Serialize};
use std::ops::{Add, Mul, Sub};

/// Flat index of (l, m), m in −l..=l.
#[inline]
pub fn idx(l: usize, m: i64) -> usize {
    (l * l) as usize + (l as i64 + m) as usize
}

#[inline]
pub fn n_coeffs(lmax: usize) -> usize {
    (lmax + 1) * (lmax + 1)
}

/// (l, m) for a flat index.
pub fn lm(i: usize) -> (usize, i64) {
    let l = (i as f64).sqrt() as usize;
    let l = if (l + 1) * (l + 1) <= i { l + 1 } else { l };
    (l, i as i64 - (l * l) as i64 - l as i64)
}

trait Num: Copy + Add<Output = Self> + Sub<Output = Self> + Mul<Output = Self> + Mul<f64, Output = Self> {
    fn zero() -> Self;
    fn one() -> Self;
}

impl Num for f64 {
    fn zero() -> Self {
        0.0
    }
    fn one() -> Self {
        1.0
    }
}

/// Value with gradient (forward-mode dual number in 3 variables).
#[derive(Clone, Copy, Debug)]
struct Dual {
    v: f64,
    g: V3,
}

impl Add for Dual {
    type Output = Dual;
    fn add(self, o: Dual) -> Dual {
        Dual { v: self.v + o.v, g: [self.g[0] + o.g[0], self.g[1] + o.g[1], self.g[2] + o.g[2]] }
    }
}
impl Sub for Dual {
    type Output = Dual;
    fn sub(self, o: Dual) -> Dual {
        Dual { v: self.v - o.v, g: [self.g[0] - o.g[0], self.g[1] - o.g[1], self.g[2] - o.g[2]] }
    }
}
impl Mul for Dual {
    type Output = Dual;
    fn mul(self, o: Dual) -> Dual {
        let g = |k: usize| self.v * o.g[k] + o.v * self.g[k];
        Dual { v: self.v * o.v, g: [g(0), g(1), g(2)] }
    }
}
impl Mul<f64> for Dual {
    type Output = Dual;
    fn mul(self, s: f64) -> Dual {
        Dual { v: self.v * s, g: [self.g[0] * s, self.g[1] * s, self.g[2] * s] }
    }
}
impl Num for Dual {
    fn zero() -> Self {
        Dual { v: 0.0, g: [0.0; 3] }
    }
    fn one() -> Self {
        Dual { v: 1.0, g: [0.0; 3] }
    }
}

/// Value, gradient and Hessian (packed xx, xy, xz, yy, yz, zz) in 3 variables.
#[derive(Clone, Copy, Debug)]
struct Dual2 {
    v: f64,
    g: V3,
    h: [f64; 6],
}

const HIDX: [(usize, usize); 6] = [(0, 0), (0, 1), (0, 2), (1, 1), (1, 2), (2, 2)];

impl Add for Dual2 {
    type Output = Dual2;
    fn add(self, o: Dual2) -> Dual2 {
        Dual2 { v: self.v + o.v, g: std::array::from_fn(|k| self.g[k] + o.g[k]), h: std::array::from_fn(|k| self.h[k] + o.h[k]) }
    }
}
impl Sub for Dual2 {
    type Output = Dual2;
    fn sub(self, o: Dual2) -> Dual2 {
        Dual2 { v: self.v - o.v, g: std::array::from_fn(|k| self.g[k] - o.g[k]), h: std::array::from_fn(|k| self.h[k] - o.h[k]) }
    }
}
impl Mul for Dual2 {
    type Output = Dual2;
    fn mul(self, o: Dual2) -> Dual2 {
        Dual2 {
            v: self.v * o.v,
            g: std::array::from_fn(|k| self.v * o.g[k] + o.v * self.g[k]),
            h: std::array::from_fn(|k| {
                let (i, j) = HIDX[k];
                self.v * o.h[k] + o.v * self.h[k] + self.g[i] * o.g[j] + self.g[j] * o.g[i]
            }),
        }
    }
}
impl Mul<f64> for Dual2 {
    type Output = Dual2;
    fn mul(self, s: f64) -> Dual2 {
        Dual2 { v: self.v * s, g: self.g.map(|x| x * s), h: self.h.map(|x| x * s) }
    }
}
impl Num for Dual2 {
    fn zero() -> Self {
        Dual2 { v: 0.0, g: [0.0; 3], h: [0.0; 6] }
    }
    fn one() -> Self {
        Dual2 { v: 1.0, g: [0.0; 3], h: [0.0; 6] }
    }
}

/// N_lm = sqrt(2 (l−m)!/(l+m)!) for m > 0, 1 for m = 0 (tabulated up to l = 64: it is
/// needed for every (l, m) of every harmonic evaluation).
fn norm_factor(l: usize, m: usize) -> f64 {
    const L: usize = 64;
    static TABLE: std::sync::OnceLock<Vec<f64>> = std::sync::OnceLock::new();
    if l <= L {
        let t = TABLE.get_or_init(|| (0..=L).flat_map(|l| (0..=l).map(move |m| norm_factor_direct(l, m))).collect());
        return t[l * (l + 1) / 2 + m];
    }
    norm_factor_direct(l, m)
}

fn norm_factor_direct(l: usize, m: usize) -> f64 {
    if m == 0 {
        return 1.0;
    }
    let mut p = 1.0;
    for k in (l - m + 1)..=(l + m) {
        p *= k as f64;
    }
    (2.0 / p).sqrt()
}

fn regular_generic<T: Num>(lmax: usize, x: T, y: T, z: T, out: &mut [T]) {
    debug_assert!(out.len() >= n_coeffs(lmax));
    // Q_l^m = r^l P_l^m(cos θ) e^{imφ} (no Condon–Shortley phase), stored as (re, im)
    // for the current m column: Q_m^m = (2m−1)(x+iy) Q_{m−1}^{m−1}, Q_{m+1}^m = (2m+1) z Q_m^m,
    // (l−m) Q_l^m = (2l−1) z Q_{l−1}^m − (l+m−1) r² Q_{l−2}^m.
    let r2 = x * x + y * y + z * z;
    let (mut mm_re, mut mm_im) = (T::one(), T::zero());
    for m in 0..=lmax {
        if m > 0 {
            let f = (2 * m - 1) as f64;
            let re = (x * mm_re - y * mm_im) * f;
            let im = (x * mm_im + y * mm_re) * f;
            mm_re = re;
            mm_im = im;
        }
        let store = |out: &mut [T], l: usize, re: T, im: T| {
            let n = norm_factor(l, m);
            out[idx(l, m as i64)] = re * n;
            if m > 0 {
                out[idx(l, -(m as i64))] = im * n;
            }
        };
        store(out, m, mm_re, mm_im);
        let (mut q2_re, mut q2_im) = (T::zero(), T::zero()); // Q_{l-2}^m
        let (mut q1_re, mut q1_im) = (mm_re, mm_im); // Q_{l-1}^m
        for l in (m + 1)..=lmax {
            let a = (2 * l - 1) as f64;
            let b = (l + m - 1) as f64;
            let inv = 1.0 / (l - m) as f64;
            let re = (z * q1_re * a - r2 * q2_re * b) * inv;
            let im = (z * q1_im * a - r2 * q2_im * b) * inv;
            (q2_re, q2_im, q1_re, q1_im) = (q1_re, q1_im, re, im);
            store(out, l, re, im);
        }
    }
}

/// Y_lm(r) for all l ≤ lmax.
pub fn regular(lmax: usize, r: V3, out: &mut [f64]) {
    regular_generic(lmax, r[0], r[1], r[2], out);
}

/// Y_lm(r) and ∇Y_lm(r).
pub fn regular_grad(lmax: usize, r: V3) -> (Vec<f64>, Vec<V3>) {
    let n = n_coeffs(lmax);
    let mut out = vec![Dual::zero(); n];
    let d = |k: usize, v: f64| {
        let mut g = [0.0; 3];
        g[k] = 1.0;
        Dual { v, g }
    };
    regular_generic(lmax, d(0, r[0]), d(1, r[1]), d(2, r[2]), &mut out);
    (out.iter().map(|x| x.v).collect(), out.iter().map(|x| x.g).collect())
}

/// Y_lm(r) with gradients and Hessians.
fn regular_hess(lmax: usize, r: V3) -> Vec<Dual2> {
    let mut out = vec![Dual2::zero(); n_coeffs(lmax)];
    let d = |k: usize, v: f64| {
        let mut g = [0.0; 3];
        g[k] = 1.0;
        Dual2 { v, g, h: [0.0; 6] }
    };
    regular_generic(lmax, d(0, r[0]), d(1, r[1]), d(2, r[2]), &mut out);
    out
}

/// Expansion of a potential about `center`; `coeffs[idx(l, m)]` in volts (lengths scaled by r0).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Expansion {
    pub lmax: usize,
    pub center: V3,
    /// Length scale (m).
    pub r0: f64,
    pub coeffs: Vec<f64>,
}

/// Cartesian Taylor data at the expansion center, from the l ≤ 2 coefficients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Local {
    /// Potential (V).
    pub phi: f64,
    /// ∇φ (V/m); the electric field is −∇φ.
    pub grad: V3,
    /// ∂²φ/∂x_i∂x_j (V/m²); traceless.
    pub hessian: [[f64; 3]; 3],
}

impl Expansion {
    fn scaled(&self, r: V3) -> V3 {
        let d = sub(r, self.center);
        [d[0] / self.r0, d[1] / self.r0, d[2] / self.r0]
    }

    pub fn potential(&self, r: V3) -> f64 {
        let mut y = vec![0.0; n_coeffs(self.lmax)];
        regular(self.lmax, self.scaled(r), &mut y);
        y.iter().zip(&self.coeffs).map(|(a, b)| a * b).sum()
    }

    /// (φ, ∇φ) at r.
    pub fn potential_grad(&self, r: V3) -> (f64, V3) {
        let (y, g) = regular_grad(self.lmax, self.scaled(r));
        let mut p = 0.0;
        let mut grad = [0.0; 3];
        for i in 0..y.len() {
            p += self.coeffs[i] * y[i];
            for k in 0..3 {
                grad[k] += self.coeffs[i] * g[i][k] / self.r0;
            }
        }
        (p, grad)
    }

    /// φ, ∇φ (V/m) and ∂²φ/∂x_i∂x_j (V/m²) at r.
    pub fn derivatives(&self, r: V3) -> Local {
        let y = regular_hess(self.lmax, self.scaled(r));
        let mut acc = Dual2::zero();
        for (yi, &c) in y.iter().zip(&self.coeffs) {
            acc = acc + *yi * c;
        }
        let (r1, r2) = (1.0 / self.r0, 1.0 / (self.r0 * self.r0));
        let mut hessian = [[0.0; 3]; 3];
        for (k, &(i, j)) in HIDX.iter().enumerate() {
            hessian[i][j] = acc.h[k] * r2;
            hessian[j][i] = acc.h[k] * r2;
        }
        Local { phi: acc.v, grad: acc.g.map(|g| g * r1), hessian }
    }

    pub fn local(&self) -> Local {
        let a = |l: usize, m: i64| if l <= self.lmax { self.coeffs[idx(l, m)] } else { 0.0 };
        let s3 = 3f64.sqrt();
        let r1 = 1.0 / self.r0;
        let r2 = r1 * r1;
        let (a20, a21, b21, a22, b22) = (a(2, 0), a(2, 1), a(2, -1), a(2, 2), a(2, -2));
        let hxx = (-a20 + s3 * a22) * r2;
        let hyy = (-a20 - s3 * a22) * r2;
        let hzz = 2.0 * a20 * r2;
        let hxy = s3 * b22 * r2;
        let hxz = s3 * a21 * r2;
        let hyz = s3 * b21 * r2;
        Local {
            phi: a(0, 0),
            grad: [a(1, 1) * r1, a(1, -1) * r1, a(1, 0) * r1],
            hessian: [[hxx, hxy, hxz], [hxy, hyy, hyz], [hxz, hyz, hzz]],
        }
    }

    /// Linear combination Σ w_k E_k of expansions with the same center, r0 and lmax.
    pub fn combine<'a>(items: impl IntoIterator<Item = (f64, &'a Expansion)>) -> Option<Expansion> {
        let mut out: Option<Expansion> = None;
        for (w, e) in items {
            let o = out.get_or_insert_with(|| Expansion { coeffs: vec![0.0; e.coeffs.len()], ..e.clone() });
            for (c, x) in o.coeffs.iter_mut().zip(&e.coeffs) {
                *c += w * x;
            }
        }
        out
    }
}

/// Per-panel moments  G[j][lm] = (1 / 4π r0) ∫_T_j Y_lm(ρ) / |ρ|^{2l+1} dA,  ρ = (r' − c)/r0,
/// so that for surface density u = σ/ε0 (V/m) the expansion coefficients are a = Σ_j u_j G[j].
/// Panels close to the center relative to their size are subdivided.
pub fn panel_moments(p: &Panel, center: V3, r0: f64, lmax: usize, out: &mut [f64]) {
    out.iter_mut().for_each(|x| *x = 0.0);
    let ratio = (20.0f64).max(2.0 * (lmax as f64 + 1.0));
    let mut y = vec![0.0; n_coeffs(lmax)];
    accumulate(p, center, r0, lmax, ratio, 8, &mut y, out);
    let s = 1.0 / (4.0 * std::f64::consts::PI * r0);
    out.iter_mut().for_each(|x| *x *= s);
}

#[allow(clippy::too_many_arguments)]
fn accumulate(p: &Panel, c: V3, r0: f64, lmax: usize, ratio: f64, depth: u32, y: &mut [f64], out: &mut [f64]) {
    let d = sub(p.centroid, c);
    if depth > 0 && dot(d, d) < (ratio * p.radius).powi(2) {
        for q in p.split4() {
            accumulate(&q, c, r0, lmax, ratio, depth - 1, y, out);
        }
        return;
    }
    for &(l0, l1, w) in &RADON7 {
        let x = sub(p.point(l0, l1), c);
        let rho = [x[0] / r0, x[1] / r0, x[2] / r0];
        let rr = dot(rho, rho).sqrt();
        regular(lmax, rho, y);
        let wa = w * p.area;
        let inv = 1.0 / rr;
        let mut f = wa * inv; // w·A / |ρ|^{2l+1}
        let inv2 = inv * inv;
        for l in 0..=lmax {
            for i in (l * l)..((l + 1) * (l + 1)) {
                out[i] += f * y[i];
            }
            f *= inv2;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_roundtrip() {
        for i in 0..n_coeffs(12) {
            let (l, m) = lm(i);
            assert_eq!(idx(l, m), i);
            assert!(m.unsigned_abs() as usize <= l);
        }
    }

    #[test]
    fn low_orders_explicit() {
        let r = [0.3, -0.7, 0.45];
        let [x, y, z] = r;
        let mut v = vec![0.0; n_coeffs(2)];
        regular(2, r, &mut v);
        let s3 = 3f64.sqrt();
        let expect = [
            (idx(0, 0), 1.0),
            (idx(1, 1), x),
            (idx(1, -1), y),
            (idx(1, 0), z),
            (idx(2, 0), z * z - 0.5 * (x * x + y * y)),
            (idx(2, 1), s3 * x * z),
            (idx(2, -1), s3 * y * z),
            (idx(2, 2), 0.5 * s3 * (x * x - y * y)),
            (idx(2, -2), s3 * x * y),
        ];
        for (i, e) in expect {
            assert!((v[i] - e).abs() < 1e-14, "{:?}: {} vs {e}", lm(i), v[i]);
        }
    }

    #[test]
    fn addition_theorem() {
        let r = [0.2, -0.1, 0.3];
        let rp = [0.9, 0.8, -1.1];
        let lmax = 40;
        let mut a = vec![0.0; n_coeffs(lmax)];
        let mut b = vec![0.0; n_coeffs(lmax)];
        regular(lmax, r, &mut a);
        regular(lmax, rp, &mut b);
        let rr = dot(rp, rp).sqrt();
        let mut s = 0.0;
        for l in 0..=lmax {
            let f = rr.powi(-(2 * l as i32 + 1));
            for i in (l * l)..((l + 1) * (l + 1)) {
                s += a[i] * b[i] * f;
            }
        }
        let exact = 1.0 / dot(sub(r, rp), sub(r, rp)).sqrt();
        assert!((s - exact).abs() < 1e-12, "{s} vs {exact}");
    }

    #[test]
    fn harmonic_and_gradient() {
        let lmax = 8;
        let r = [0.31, 0.22, -0.4];
        let (v, g) = regular_grad(lmax, r);
        let h = 1e-5;
        let eval = |p: V3| {
            let mut o = vec![0.0; n_coeffs(lmax)];
            regular(lmax, p, &mut o);
            o
        };
        let mut lap = vec![0.0; v.len()];
        for k in 0..3 {
            let mut a = r;
            let mut b = r;
            a[k] += h;
            b[k] -= h;
            let (fa, fb) = (eval(a), eval(b));
            for i in 0..v.len() {
                let fd = (fa[i] - fb[i]) / (2.0 * h);
                assert!((fd - g[i][k]).abs() < 1e-7, "grad {:?} d{k}: {} vs {fd}", lm(i), g[i][k]);
                lap[i] += (fa[i] - 2.0 * v[i] + fb[i]) / (h * h);
            }
        }
        for (i, l) in lap.iter().enumerate() {
            assert!(l.abs() < 1e-3, "Laplacian of {:?} = {l}", lm(i));
        }
    }

    #[test]
    fn local_matches_finite_differences() {
        let e = Expansion { lmax: 4, center: [1e-4, 0.0, -2e-4], r0: 5e-4, coeffs: (0..25).map(|i| ((i * 7 % 11) as f64 - 5.0) * 0.1).collect() };
        let loc = e.local();
        let c = e.center;
        let h = 1e-7;
        let grad = |p: V3| e.potential_grad(p).1;
        let scale = loc.hessian.iter().flatten().fold(0.0f64, |m, x| m.max(x.abs()));
        for i in 0..3 {
            let mut a = c;
            let mut b = c;
            a[i] += h;
            b[i] -= h;
            let (ga, gb) = (grad(a), grad(b));
            for j in 0..3 {
                let fd = (ga[j] - gb[j]) / (2.0 * h);
                assert!((fd - loc.hessian[i][j]).abs() < 1e-6 * scale, "H[{i}][{j}] {} vs {fd}", loc.hessian[i][j]);
            }
        }
        let g = e.potential_grad(c).1;
        for k in 0..3 {
            assert!((g[k] - loc.grad[k]).abs() < 1e-9 * g[k].abs().max(1.0));
        }
        // second-order jets away from the center agree with finite differences of the gradient
        let p = [c[0] + 1.3e-4, c[1] - 0.7e-4, c[2] + 2.1e-4];
        let d = e.derivatives(p);
        let (phi, g) = e.potential_grad(p);
        assert!((d.phi - phi).abs() < 1e-12 * phi.abs().max(1.0));
        for k in 0..3 {
            assert!((d.grad[k] - g[k]).abs() < 1e-9 * g[k].abs().max(1.0));
        }
        for i in 0..3 {
            let mut a = p;
            let mut b = p;
            a[i] += h;
            b[i] -= h;
            let (ga, gb) = (grad(a), grad(b));
            for j in 0..3 {
                let fd = (ga[j] - gb[j]) / (2.0 * h);
                assert!((fd - d.hessian[i][j]).abs() < 1e-6 * scale, "off-center H[{i}][{j}] {} vs {fd}", d.hessian[i][j]);
            }
        }
    }
}
