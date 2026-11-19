//! Geometric queries on triangle meshes: exact point-to-surface distances.

/// Closest point on triangle (a, b, c) to p (Ericson, Real-Time Collision Detection 5.1.5).
pub fn closest_point_on_triangle(p: [f64; 3], a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 3] {
    let sub = |u: [f64; 3], v: [f64; 3]| [u[0] - v[0], u[1] - v[1], u[2] - v[2]];
    let dot = |u: [f64; 3], v: [f64; 3]| u[0] * v[0] + u[1] * v[1] + u[2] * v[2];
    let add_s = |u: [f64; 3], v: [f64; 3], s: f64| [u[0] + s * v[0], u[1] + s * v[1], u[2] + s * v[2]];
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = sub(p, b);
    let (d3, d4) = (dot(ab, bp), dot(ac, bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return add_s(a, ab, d1 / (d1 - d3));
    }
    let cp = sub(p, c);
    let (d5, d6) = (dot(ab, cp), dot(ac, cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return add_s(a, ac, d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return add_s(b, sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    let (v, w) = (vb * denom, vc * denom);
    add_s(add_s(a, ab, v), ac, w)
}

/// For each group (body), the closest point of its triangles to `p`:
/// returns (distance, closest point) per group, `None` for groups without triangles.
pub fn closest_per_group(
    p: [f64; 3],
    positions: &[f64],
    triangles: &[u32],
    tri_group: &[u32],
    n_groups: usize,
) -> Vec<Option<(f64, [f64; 3])>> {
    let v = |i: u32| {
        let i = 3 * i as usize;
        [positions[i], positions[i + 1], positions[i + 2]]
    };
    let mut best: Vec<Option<(f64, [f64; 3])>> = vec![None; n_groups];
    for (t, tri) in triangles.chunks_exact(3).enumerate() {
        let q = closest_point_on_triangle(p, v(tri[0]), v(tri[1]), v(tri[2]));
        let d = ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2)).sqrt();
        let g = tri_group[t] as usize;
        if best[g].map_or(true, |(bd, _)| d < bd) {
            best[g] = Some((d, q));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closest_points() {
        let (a, b, c) = ([0., 0., 0.], [1., 0., 0.], [0., 1., 0.]);
        let q = closest_point_on_triangle([0.2, 0.2, 5.], a, b, c);
        assert!((q[0] - 0.2).abs() < 1e-12 && (q[1] - 0.2).abs() < 1e-12 && q[2] == 0.);
        assert_eq!(closest_point_on_triangle([-1., -1., 0.], a, b, c), a);
        assert_eq!(closest_point_on_triangle([2., -1., 0.], a, b, c), b);
        let q = closest_point_on_triangle([1., 1., 0.], a, b, c);
        assert!((q[0] - 0.5).abs() < 1e-12 && (q[1] - 0.5).abs() < 1e-12);
    }
}
