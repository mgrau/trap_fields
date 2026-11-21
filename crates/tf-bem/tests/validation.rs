//! Physics checks against analytic electrostatics.
use std::f64::consts::PI;
use tf_bem::harmonics::idx;
use tf_bem::{evaluate, evaluate_many, expand, moment_table, solve, Conductor, Problem, EPS0};

/// Icosphere: (nodes, triangles) with outward orientation.
fn icosphere(center: [f64; 3], radius: f64, subdiv: u32) -> (Vec<f64>, Vec<u32>) {
    let t = (1.0 + 5f64.sqrt()) / 2.0;
    let mut v: Vec<[f64; 3]> = vec![
        [-1., t, 0.], [1., t, 0.], [-1., -t, 0.], [1., -t, 0.],
        [0., -1., t], [0., 1., t], [0., -1., -t], [0., 1., -t],
        [t, 0., -1.], [t, 0., 1.], [-t, 0., -1.], [-t, 0., 1.],
    ];
    let mut f: Vec<[u32; 3]> = vec![
        [0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11], [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
        [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9], [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1],
    ];
    let unit = |p: [f64; 3]| {
        let n = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        [p[0] / n, p[1] / n, p[2] / n]
    };
    v.iter_mut().for_each(|p| *p = unit(*p));
    for _ in 0..subdiv {
        let mut cache = std::collections::HashMap::new();
        let mut mid = |a: u32, b: u32, v: &mut Vec<[f64; 3]>| {
            *cache.entry((a.min(b), a.max(b))).or_insert_with(|| {
                let (p, q) = (v[a as usize], v[b as usize]);
                v.push(unit([(p[0] + q[0]) / 2., (p[1] + q[1]) / 2., (p[2] + q[2]) / 2.]));
                v.len() as u32 - 1
            })
        };
        let mut nf = Vec::with_capacity(f.len() * 4);
        for [a, b, c] in f {
            let (ab, bc, ca) = (mid(a, b, &mut v), mid(b, c, &mut v), mid(c, a, &mut v));
            nf.extend([[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        f = nf;
    }
    let nodes = v.iter().flat_map(|p| [center[0] + radius * p[0], center[1] + radius * p[1], center[2] + radius * p[2]]).collect();
    (nodes, f.into_iter().flatten().collect())
}

fn merge(parts: &[(Vec<f64>, Vec<u32>)]) -> (Vec<f64>, Vec<u32>, Vec<usize>) {
    let (mut nodes, mut tris, mut cond) = (vec![], vec![], vec![]);
    for (k, (n, t)) in parts.iter().enumerate() {
        let off = (nodes.len() / 3) as u32;
        nodes.extend_from_slice(n);
        tris.extend(t.iter().map(|i| i + off));
        cond.extend(std::iter::repeat(k).take(t.len() / 3));
    }
    (nodes, tris, cond)
}

fn cond(name: &str, floating: bool) -> Conductor {
    Conductor { name: name.into(), floating }
}

#[test]
fn isolated_sphere() {
    let r = 1e-3;
    let (nodes, tris) = icosphere([0.0; 3], r, 4); // 5120 panels
    let p = Problem::from_mesh(&nodes, &tris, vec![0; tris.len() / 3], vec![cond("s", false)]).unwrap();
    let s = solve(&p, &|_| {}).unwrap();
    println!("sphere: {} panels, {:?}", p.panels.len(), s.timings);

    // C = 4π ε0 R. The inscribed polyhedron is slightly smaller than the sphere.
    let c = s.charge[0][0];
    let exact = 4.0 * PI * EPS0 * r;
    println!("  C = {c:e} F, exact {exact:e}, rel {:.2e}", c / exact - 1.0);
    assert!((c / exact - 1.0).abs() < 2e-3);

    // potential: 1 V inside, R/|x| outside
    let pts = [[0.0, 0.0, 0.0], [0.3e-3, -0.2e-3, 0.1e-3], [2e-3, 0.0, 0.0], [0.0, 3e-3, 4e-3]];
    let ev = evaluate(&p.panels, &s.u[0], &pts);
    for (x, (phi, g)) in pts.iter().zip(&ev) {
        let d = (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt();
        let exact = if d < r { 1.0 } else { r / d };
        println!("  phi{x:?} = {phi:.6}  (exact {exact:.6}), |grad| {:.3e}", (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt());
        assert!((phi - exact).abs() < 3e-3, "phi at {x:?}");
        if d < r {
            assert!(g.iter().all(|c| c.abs() < 2.0), "field inside a conductor: {g:?} V/m");
        }
    }
}

#[test]
fn floating_sphere_is_neutral_and_takes_potential() {
    // driven sphere A (1 V) and floating sphere B far away: B carries no net charge and
    // sits at ≈ the potential of A's field at its center, R_A/d (+ O(R³/d³))
    let (ra, rb, d) = (1e-3, 0.5e-3, 10e-3);
    let (nodes, tris, c) = merge(&[icosphere([0.0; 3], ra, 3), icosphere([d, 0.0, 0.0], rb, 3)]);
    let p = Problem::from_mesh(&nodes, &tris, c, vec![cond("A", false), cond("B", true)]).unwrap();
    let s = solve(&p, &|_| {}).unwrap();
    assert_eq!(s.basis, vec![0]);
    let qb = s.charge[0][1];
    let qa = s.charge[0][0];
    let vb = s.potential[0][1];
    println!("floating: qA {qa:e}, qB {qb:e}, VB {vb:.5} (≈ {:.5})", ra / d);
    assert!(qb.abs() < 1e-9 * qa.abs().max(1e-30) * 1e6, "floating charge {qb:e}");
    assert!((vb - ra / d).abs() < 0.03 * ra / d, "floating potential {vb}");
    // B's surface is an equipotential at vb
    let ev = evaluate(&p.panels, &s.u[0], &[[d, 0.0, 0.0], [d + 0.2e-3, 0.1e-3, 0.0]]);
    for (phi, _) in ev {
        assert!((phi - vb).abs() < 1e-3 * vb.abs() + 1e-5, "inside floating conductor {phi} vs {vb}");
    }
}

#[test]
fn evaluate_many_matches_evaluate_per_density() {
    // two driven spheres: one kernel pass for both basis densities gives what one pass each does
    let (nodes, tris, c) = merge(&[icosphere([0.0; 3], 1e-3, 2), icosphere([4e-3, 0.0, 0.0], 0.5e-3, 2)]);
    let p = Problem::from_mesh(&nodes, &tris, c, vec![cond("A", false), cond("B", false)]).unwrap();
    let s = solve(&p, &|_| {}).unwrap();
    let pts = [[2e-3, 1e-3, 0.0], [0.0, 0.0, 0.0], [-3e-3, 0.5e-3, 2e-3], [4e-3, 0.1e-3, 0.0]];
    let many = evaluate_many(&p.panels, &[&s.u[0], &s.u[1]], &pts);
    for b in 0..2 {
        let one = evaluate(&p.panels, &s.u[b], &pts);
        for (k, (phi, g)) in one.iter().enumerate() {
            let (pm, gm) = many[k][b];
            assert!((pm - phi).abs() <= 1e-12 * phi.abs().max(1.0), "phi {pm} vs {phi}");
            for i in 0..3 {
                assert!((gm[i] - g[i]).abs() <= 1e-12 * g[i].abs().max(1.0), "grad {gm:?} vs {g:?}");
            }
        }
    }
}

#[test]
fn capacitance_matrix_two_spheres() {
    // 2 × 5120 panels: about the size of a trap mesh, so this doubles as a timing check
    let (r, d) = (1e-3, 4e-3);
    let (nodes, tris, c) = merge(&[icosphere([0.0; 3], r, 4), icosphere([d, 0.0, 0.0], r, 4)]);
    let p = Problem::from_mesh(&nodes, &tris, c, vec![cond("A", false), cond("B", false)]).unwrap();
    let s = solve(&p, &|_| {}).unwrap();
    println!("two spheres: {} panels, {:?}", p.panels.len(), s.timings);
    let cm = &s.charge;
    println!("C = {cm:?}");
    // symmetric (reciprocity), negative off-diagonal, diagonal > isolated sphere
    assert!((cm[0][1] - cm[1][0]).abs() < 2e-3 * cm[0][1].abs());
    assert!(cm[0][1] < 0.0 && cm[0][0] > 4.0 * PI * EPS0 * r);
    // exact (image series = Smythe's bispherical sums): C11 = 1.07182, C12 = −0.26921 in units of 4πε0 R
    let c11 = cm[0][0] / (4.0 * PI * EPS0 * r);
    let c12 = cm[0][1] / (4.0 * PI * EPS0 * r);
    let (e11, e12) = image_series(r, d);
    println!("C11 {c11:.5} (series {e11:.5}), C12 {c12:.5} (series {e12:.5})");
    assert!((c11 / e11 - 1.0).abs() < 1.5e-3 && (c12 / e12 - 1.0).abs() < 2e-3);
}

/// Capacitance coefficients of two equal spheres (radius a, center distance d) in units
/// of 4πε0 a, from the classic image-charge recursion.
fn image_series(a: f64, d: f64) -> (f64, f64) {
    // sphere 1 at 1 V, sphere 2 at 0 V: alternate images
    // charges on sphere 1 (q1) and sphere 2 (q2), with positions measured from each center
    let (mut c11, mut c12) = (0.0, 0.0);
    // image of charge q at distance s from the other sphere's center: q' = -q a/s, at a²/s
    let mut q = a; // in units of 4πε0 · V: initial charge a at center of sphere 1
    let mut x = 0.0; // position measured from sphere 1 center, along +d
    let mut on_first = true;
    for _ in 0..200 {
        if on_first {
            c11 += q;
        } else {
            c12 += q;
        }
        // image in the other sphere
        let (s, other_center) = if on_first { (d - x, d) } else { (x, 0.0) };
        let qi = -q * a / s;
        let xi = if on_first { other_center - a * a / s } else { a * a / s };
        q = qi;
        x = xi;
        on_first = !on_first;
    }
    (c11 / a, c12 / a)
}

#[test]
fn expansion_from_charges_matches_direct_evaluation() {
    // a 1 V sphere off to the side: inside the ball |x| < (d − R) about the origin the
    // potential is R/|x − c|; the expansion is built from the BEM charges directly
    let (r, c) = (1e-3, [2.5e-3, 1.0e-3, -0.5e-3]);
    let (nodes, tris) = icosphere(c, r, 4);
    let p = Problem::from_mesh(&nodes, &tris, vec![0; tris.len() / 3], vec![cond("s", false)]).unwrap();
    let s = solve(&p, &|_| {}).unwrap();
    let rmin = p.distance_to_surface([0.0; 3]);
    let lmax = 12;
    let table = moment_table(&p.panels, [0.0; 3], rmin, lmax);
    let e = expand(&table, &s.u[0], [0.0; 3], rmin, lmax);

    let dist = |x: [f64; 3]| ((x[0] - c[0]).powi(2) + (x[1] - c[1]).powi(2) + (x[2] - c[2]).powi(2)).sqrt();
    let pts = [[0.0, 0.0, 0.0], [0.3e-3, 0.0, 0.0], [-0.2e-3, 0.4e-3, 0.1e-3], [0.0, 0.0, -0.5e-3]];
    let direct = evaluate(&p.panels, &s.u[0], &pts);
    for (x, (phi_d, g_d)) in pts.iter().zip(&direct) {
        let (phi_e, g_e) = e.potential_grad(*x);
        let exact = r / dist(*x);
        println!("  {x:?}: expansion {phi_e:.7}  direct {phi_d:.7}  exact {exact:.7}");
        assert!((phi_e - phi_d).abs() < 1e-5, "expansion vs direct BEM at {x:?}");
        for k in 0..3 {
            assert!((g_e[k] - g_d[k]).abs() < 1e-3 * (g_d[0].abs() + g_d[1].abs() + g_d[2].abs()), "grad {k} at {x:?}");
        }
    }
    // l = 0 and l = 1 against the point-source values at the origin (R/|c|, ∇ = R c/|c|³ ...)
    let dc = dist([0.0; 3]);
    assert!((e.coeffs[idx(0, 0)] - r / dc).abs() < 2e-3 * r / dc);
    let loc = e.local();
    for k in 0..3 {
        let exact = r * c[k] / dc.powi(3);
        assert!((loc.grad[k] - exact).abs() < 5e-3 * (r / dc / dc), "grad[{k}] {} vs {exact}", loc.grad[k]);
    }
    let tr = loc.hessian[0][0] + loc.hessian[1][1] + loc.hessian[2][2];
    assert!(tr.abs() < 1e-12 * loc.hessian[0][0].abs().max(1.0));
}

#[test]
fn compressed_solver_matches_dense() {
    use tf_bem::{solve_with, Method, SolveOptions};
    // two driven spheres and a floating one, ~9k panels
    let (nodes, tris, c) = merge(&[
        icosphere([0.0; 3], 1e-3, 4),
        icosphere([3e-3, 0.0, 0.0], 0.8e-3, 4),
        icosphere([0.0, 3.5e-3, 1e-3], 0.5e-3, 3),
    ]);
    let p = Problem::from_mesh(&nodes, &tris, c, vec![cond("A", false), cond("B", false), cond("F", true)]).unwrap();
    let dense = solve(&p, &|_| {}).unwrap();
    let opts = SolveOptions { method: Method::Compressed, ..Default::default() };
    let h = solve_with(&p, &opts, &|_| {}).unwrap();
    println!(
        "{} panels: dense {:?}; compressed {:?}, stats {:?}",
        p.panels.len(),
        dense.timings,
        h.timings,
        h.stats
    );
    for b in 0..2 {
        for k in 0..3 {
            let (qd, qh) = (dense.charge[b][k], h.charge[b][k]);
            let scale = dense.charge[b][b].abs();
            assert!((qd - qh).abs() < 1e-5 * scale, "charge[{b}][{k}]: dense {qd:e} vs compressed {qh:e}");
        }
        let (vd, vh) = (dense.potential[b][2], h.potential[b][2]);
        assert!((vd - vh).abs() < 1e-5, "floating potential: {vd} vs {vh}");
        // fields at points between the conductors
        let pts = [[1.5e-3, 0.2e-3, 0.0], [0.5e-3, 1.8e-3, 0.3e-3], [-2e-3, -1e-3, 0.5e-3]];
        let ed = evaluate(&p.panels, &dense.u[b], &pts);
        let eh = evaluate(&p.panels, &h.u[b], &pts);
        for ((pd, gd), (ph, gh)) in ed.iter().zip(&eh) {
            assert!((pd - ph).abs() < 1e-5, "potential {pd} vs {ph}");
            let gn = (gd[0].powi(2) + gd[1].powi(2) + gd[2].powi(2)).sqrt();
            for k in 0..3 {
                assert!((gd[k] - gh[k]).abs() < 1e-4 * gn, "field {gd:?} vs {gh:?}");
            }
        }
    }
    assert!(h.stats.compression.unwrap() < 0.5);
}
