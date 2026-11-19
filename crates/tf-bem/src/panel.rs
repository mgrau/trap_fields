//! Flat triangular panels with uniform (P0) surface density, and the single-layer
//! integral  S(r) = ∫_T dA' / |r − r'|  together with its gradient ∇_r S.
//!
//! Near the panel S is evaluated in closed form (Wilton et al., IEEE TAP 32, 1984);
//! farther away a 7-point degree-5 rule is used (its relative error falls as s⁻⁶ with
//! s = distance / panel radius).

pub type V3 = [f64; 3];

#[inline]
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
#[inline]
pub fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
#[inline]
pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
#[inline]
pub fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}

#[derive(Debug, Clone)]
pub struct Panel {
    pub v: [V3; 3],
    pub centroid: V3,
    /// Unit normal, (v1 − v0) × (v2 − v0) direction.
    pub normal: V3,
    pub area: f64,
    /// Largest centroid-to-vertex distance.
    pub radius: f64,
}

impl Panel {
    /// `None` for a degenerate (zero-area) triangle.
    pub fn new(a: V3, b: V3, c: V3) -> Option<Panel> {
        let cr = cross(sub(b, a), sub(c, a));
        let n2 = norm(cr);
        if !(n2 > 0.0) || !n2.is_finite() {
            return None;
        }
        let centroid = scale(add(add(a, b), c), 1.0 / 3.0);
        let radius = [a, b, c].iter().map(|&v| norm(sub(v, centroid))).fold(0.0, f64::max);
        Some(Panel { v: [a, b, c], centroid, normal: scale(cr, 1.0 / n2), area: 0.5 * n2, radius })
    }

    /// Point at barycentric coordinates (l0, l1, 1 − l0 − l1).
    #[inline]
    pub fn point(&self, l0: f64, l1: f64) -> V3 {
        let l2 = 1.0 - l0 - l1;
        let [a, b, c] = self.v;
        [l0 * a[0] + l1 * b[0] + l2 * c[0], l0 * a[1] + l1 * b[1] + l2 * c[1], l0 * a[2] + l1 * b[2] + l2 * c[2]]
    }

    /// The four sub-triangles from edge midpoints.
    pub fn split4(&self) -> [Panel; 4] {
        let [a, b, c] = self.v;
        let m = |p: V3, q: V3| scale(add(p, q), 0.5);
        let (ab, bc, ca) = (m(a, b), m(b, c), m(c, a));
        let mk = |p, q, r| Panel::new(p, q, r).expect("sub-triangle of a valid panel");
        [mk(a, ab, ca), mk(ab, b, bc), mk(ca, bc, c), mk(ab, bc, ca)]
    }
}

/// Radon's 7-point degree-5 rule: barycentric (l0, l1) and weights summing to 1.
pub const RADON7: [(f64, f64, f64); 7] = {
    const A1: f64 = 0.101_286_507_323_456_33;
    const B1: f64 = 0.797_426_985_353_087_3;
    const W1: f64 = 0.125_939_180_544_827_14;
    const A2: f64 = 0.470_142_064_105_115_05;
    const B2: f64 = 0.059_715_871_789_769_82;
    const W2: f64 = 0.132_394_152_788_506_19;
    const T: f64 = 1.0 / 3.0;
    [(T, T, 0.225), (A1, A1, W1), (A1, B1, W1), (B1, A1, W1), (A2, A2, W2), (A2, B2, W2), (B2, A2, W2)]
};

/// Closed form below this distance / panel-radius ratio, quadrature above.
pub const NEAR_RATIO: f64 = 6.0;

/// Exact ∫_T dA'/|r − r'| and its gradient with respect to r.
pub fn analytic(p: &Panel, r: V3) -> (f64, V3) {
    let n = p.normal;
    let h = dot(n, sub(r, p.v[0]));
    let ah = h.abs();
    let rho = sub(r, scale(n, h));
    let mut pot = 0.0;
    let mut omega = 0.0; // signed solid angle / sign(h)
    let mut g = [0.0; 3];
    for i in 0..3 {
        let a = p.v[i];
        let b = p.v[(i + 1) % 3];
        let e = sub(b, a);
        let len = norm(e);
        let lhat = scale(e, 1.0 / len);
        let uhat = cross(lhat, n); // in-plane outward edge normal
        let d = sub(a, rho);
        let t0 = dot(d, uhat); // > 0 when rho is on the inner side of this edge
        let lm = dot(d, lhat);
        let lp = lm + len;
        let r0sq = t0 * t0 + h * h;
        let rp = (r0sq + lp * lp).sqrt();
        let rm = (r0sq + lm * lm).sqrt();
        let tiny = 1e-24 * len * len;
        // ln((R+ + l+)/(R- + l-)), written to avoid cancellation when l < 0
        let f = if r0sq <= tiny {
            0.0
        } else {
            let gp = if lp >= 0.0 { rp + lp } else { r0sq / (rp - lp) };
            let gm = if lm >= 0.0 { rm + lm } else { r0sq / (rm - lm) };
            (gp / gm).ln()
        };
        let beta = if t0 * t0 <= tiny { 0.0 } else { (t0 * lp).atan2(r0sq + ah * rp) - (t0 * lm).atan2(r0sq + ah * rm) };
        pot += t0 * f;
        omega += beta;
        g = sub(g, scale(uhat, f));
    }
    pot -= ah * omega;
    // the normal derivative jumps by 4π·(unit density) across the panel; on the plane
    // itself return the principal value (no normal component)
    if ah > 1e-12 * p.radius {
        g = sub(g, scale(n, h.signum() * omega));
    }
    (pot, g)
}

/// ∫_T dA'/|r − r'| by the 7-point rule (accurate only away from the panel).
#[inline]
pub fn quadrature(p: &Panel, r: V3) -> f64 {
    let mut s = 0.0;
    for &(l0, l1, w) in &RADON7 {
        let d = sub(r, p.point(l0, l1));
        s += w / dot(d, d).sqrt();
    }
    s * p.area
}

#[inline]
fn quadrature_grad(p: &Panel, r: V3) -> (f64, V3) {
    let mut s = 0.0;
    let mut g = [0.0; 3];
    for &(l0, l1, w) in &RADON7 {
        let d = sub(r, p.point(l0, l1));
        let inv = 1.0 / dot(d, d).sqrt();
        s += w * inv;
        g = sub(g, scale(d, w * inv * inv * inv));
    }
    (s * p.area, scale(g, p.area))
}

#[inline]
fn is_near(p: &Panel, r: V3) -> bool {
    let d = sub(r, p.centroid);
    dot(d, d) < (NEAR_RATIO * p.radius).powi(2)
}

/// ∫_T dA'/|r − r'|, closed form near the panel and quadrature far from it.
#[inline]
pub fn single_layer(p: &Panel, r: V3) -> f64 {
    if is_near(p, r) {
        analytic(p, r).0
    } else {
        quadrature(p, r)
    }
}

/// `single_layer` and its gradient with respect to r.
#[inline]
pub fn single_layer_grad(p: &Panel, r: V3) -> (f64, V3) {
    if is_near(p, r) {
        analytic(p, r)
    } else {
        quadrature_grad(p, r)
    }
}

/// Exact closest point on a triangle (Ericson, Real-Time Collision Detection 5.1.5).
pub fn closest_point(p: &Panel, x: V3) -> V3 {
    let [a, b, c] = p.v;
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(x, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = sub(x, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return add(a, scale(ab, d1 / (d1 - d3)));
    }
    let cp = sub(x, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return add(a, scale(ac, d2 / (d2 - d6)));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return add(b, scale(sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6))));
    }
    let denom = 1.0 / (va + vb + vc);
    add(a, add(scale(ab, vb * denom), scale(ac, vc * denom)))
}

/// Distance t > 0 along the unit ray o + t·d to the panel, if it is hit (Möller–Trumbore).
#[inline]
pub fn ray_hit(p: &Panel, o: V3, d: V3) -> Option<f64> {
    let [a, b, c] = p.v;
    let e1 = sub(b, a);
    let e2 = sub(c, a);
    let pv = cross(d, e2);
    let det = dot(e1, pv);
    if det.abs() < 1e-30 {
        return None;
    }
    let inv = 1.0 / det;
    let tv = sub(o, a);
    let u = dot(tv, pv) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let qv = cross(tv, e1);
    let v = dot(d, qv) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = dot(e2, qv) * inv;
    (t > 0.0).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference: recursive subdivision + 7-point rule, refined near r.
    fn reference(p: &Panel, r: V3, depth: u32) -> f64 {
        let d = norm(sub(r, p.centroid));
        if depth == 0 || d > 40.0 * p.radius {
            return quadrature(p, r);
        }
        p.split4().iter().map(|q| reference(q, r, depth - 1)).sum()
    }

    fn tri() -> Panel {
        Panel::new([0.1, -0.2, 0.05], [1.3, 0.1, -0.1], [0.4, 0.9, 0.2]).unwrap()
    }

    #[test]
    fn analytic_matches_refined_quadrature() {
        let p = tri();
        let pts = [
            [0.5, 0.3, 0.7],    // above
            [0.5, 0.3, -0.05],  // just below
            [2.0, 1.5, 0.3],    // off to the side
            [-0.8, 0.2, 0.0],   // roughly in-plane, outside
            [0.7, -0.6, 0.12],  // beyond an edge
            [5.0, -3.0, 4.0],   // far
            [1.3, 0.1, -0.05],  // above a vertex
        ];
        for r in pts {
            let (a, _) = analytic(&p, r);
            let q = reference(&p, r, 9);
            assert!(((a - q) / q).abs() < 1e-7, "at {r:?}: analytic {a} vs reference {q}");
        }
        // in-plane points (self terms): polar integration about the point, edge by edge,
        // ∫ dθ (distance to the edge along θ), with Simpson's rule
        for r in [p.centroid, p.point(0.7, 0.2), p.point(0.05, 0.5)] {
            let (a, g) = analytic(&p, r);
            let mut q = 0.0;
            for i in 0..3 {
                let (va, vb) = (sub(p.v[i], r), sub(p.v[(i + 1) % 3], r));
                let n = 4000;
                let f = |t: f64| norm(add(scale(va, 1.0 - t), scale(vb, t))); // distance at param t
                // dθ = |va × vb| / |x(t)|² dt along the edge, integrand ρ(θ) = |x(t)|
                let jac = norm(cross(va, vb));
                let mut s = 0.0;
                for k in 0..=n {
                    let t = k as f64 / n as f64;
                    let w = if k == 0 || k == n { 1.0 } else if k % 2 == 1 { 4.0 } else { 2.0 };
                    s += w * jac / f(t);
                }
                q += s / (3.0 * n as f64);
            }
            assert!(((a - q) / q).abs() < 1e-9, "in-plane {r:?}: {a} vs {q}");
            assert!(dot(g, p.normal).abs() < 1e-12, "principal-value normal derivative");
        }
    }

    #[test]
    fn gradient_matches_finite_differences() {
        let p = tri();
        for r in [[0.5, 0.3, 0.7], [0.5, 0.3, -0.05], [2.0, 1.5, 0.3], [0.7, -0.6, 0.12], [-0.3, 0.4, 0.4]] {
            let (_, g) = analytic(&p, r);
            let h = 1e-6;
            for k in 0..3 {
                let mut a = r;
                let mut b = r;
                a[k] += h;
                b[k] -= h;
                let fd = (analytic(&p, a).0 - analytic(&p, b).0) / (2.0 * h);
                assert!((fd - g[k]).abs() < 1e-6 * (1.0 + g[k].abs()), "d/dx{k} at {r:?}: {} vs fd {fd}", g[k]);
            }
        }
    }

    #[test]
    fn near_far_switch_is_accurate() {
        let p = tri();
        let mut worst: f64 = 0.0;
        for k in 0..200 {
            let t = k as f64 * 0.37;
            let dir = [t.cos() * (0.7 * t).sin(), t.sin() * (0.7 * t).sin(), (0.7 * t).cos()];
            let r = add(p.centroid, scale(dir, NEAR_RATIO * p.radius * 1.0001));
            let e = (quadrature(&p, r) - analytic(&p, r).0) / analytic(&p, r).0;
            worst = worst.max(e.abs());
        }
        assert!(worst < 1e-5, "quadrature error at the switch-over distance: {worst:e}");
    }

    #[test]
    fn closest_point_inside_and_vertex() {
        let p = Panel::new([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        let c = closest_point(&p, [0.2, 0.2, 3.0]);
        assert!(norm(sub(c, [0.2, 0.2, 0.0])) < 1e-15);
        assert_eq!(closest_point(&p, [-1.0, -1.0, 0.0]), [0.0, 0.0, 0.0]);
    }
}
