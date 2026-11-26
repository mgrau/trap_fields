//! The example projects under `examples/` agree with the analytic models and published
//! numbers of the designs they reproduce.
//!
//! - Ideal hyperbolic Paul trap (generated here, not an example): φ = V/2 + V(r² − 2z²)/(2r₀²)
//!   inside, q_z = 4eV/(mΩ²r₀²), radial frequency half the axial one, exact Mathieu
//!   frequencies, depth q_zV/16.
//! - Stretched ion trap of March's worked example (J. Mass Spectrom. 32, 351 (1997)):
//!   q_z = 0.450, ω_z = 167 kHz from β_z ≈ q_z/√2.
//! - Segmented four-rod trap (Miao et al., arXiv:2201.03377): the paper's Mathieu q and a,
//!   the radial frequencies and the saddle between the rods of four infinite rods.
//! - Five-wire surface trap (Daniilidis et al., NJP 13, 013032 (2011)): the RF null and
//!   the tilt of the RF principal axes against the gapless-plane model, the ion height
//!   (about 240 µm), and the published secular frequencies and mode tilt, which the RF
//!   amplitude and DC set are fitted to.

mod common;

use common::*;
use serde_json::{json, Value};
use std::f64::consts::PI;
use std::path::PathBuf;
use tf_app::demo::{self, Design};
use tf_app::App;

/// One test: gmsh must stay on the thread that first used it, and every test runs on a
/// thread of its own.
#[test]
fn examples_agree_with_theory_and_publications() {
    // generate the ideal trap before any App uses gmsh on this thread
    let ideal = temp("paul-ideal");
    demo::create_design(&ideal, Design::PaulIdeal).unwrap();
    ideal_paul_trap_is_an_ideal_quadrupole(ideal);
    march_trap_matches_the_paper();
    surface_trap_matches_the_gapless_model_and_the_paper();
    segmented_trap_matches_the_paper_and_infinite_rods();
}

/// Characteristic exponent β of Mathieu's equation x'' + (a − 2q cos 2τ) x = 0 (stable
/// region), from the monodromy matrix over one period (RK4): the exact secular
/// frequency is βΩ/2. Independent of the app's Floquet analysis.
fn mathieu_beta(a: f64, q: f64) -> f64 {
    let f = |t: f64, y: [f64; 2]| [y[1], -(a - 2.0 * q * (2.0 * t).cos()) * y[0]];
    let n = 4000;
    let h = PI / n as f64;
    let mut trace = 0.0;
    for (k, y0) in [[1.0, 0.0], [0.0, 1.0]].into_iter().enumerate() {
        let (mut y, mut t) = (y0, 0.0);
        for _ in 0..n {
            let add = |y: [f64; 2], k: [f64; 2], s: f64| [y[0] + s * k[0], y[1] + s * k[1]];
            let k1 = f(t, y);
            let k2 = f(t + h / 2.0, add(y, k1, h / 2.0));
            let k3 = f(t + h / 2.0, add(y, k2, h / 2.0));
            let k4 = f(t + h, add(y, k3, h));
            y = [0, 1].map(|i| y[i] + h / 6.0 * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]));
            t += h;
        }
        trace += y[k];
    }
    (trace / 2.0).acos() / PI
}

/// Pseudopotential (eV) of RF field amplitudes E (V/m) at drive frequency `freq`.
fn pseudo_ev(e: [f64; 3], freq: f64) -> f64 {
    let w = 2.0 * PI * freq;
    E * (e[0] * e[0] + e[1] * e[1] + e[2] * e[2]) / (4.0 * 39.96259 * AMU * w * w)
}

/// RF field (gradients of the potential) at `pts` with the RF voltages of `drive`.
fn rf_grad(app: &mut App, drive: &Value, pts: &[[f64; 3]]) -> Vec<[f64; 3]> {
    let ev = cmd(app, "eval", json!({ "points": pts, "voltages": drive["rf"] }));
    ev["grad"].as_array().unwrap().iter().map(|g| [0, 1, 2].map(|i| g[i].as_f64().unwrap())).collect()
}

/// Lowest barrier (eV) from the minimum of a 2D grid (rows j, columns i) to any node with
/// `exit(j, i)`: a priority flood over 8 neighbours, as the app's 3D depth search.
fn flood_depth(u: &[Vec<f64>], exit: impl Fn(usize, usize) -> bool) -> f64 {
    let (ny, nx) = (u.len(), u[0].len());
    let start = (0..ny).flat_map(|j| (0..nx).map(move |i| (j, i))).min_by(|a, b| u[a.0][a.1].total_cmp(&u[b.0][b.1])).unwrap();
    let mut done = vec![vec![false; nx]; ny];
    let mut heap = std::collections::BinaryHeap::new();
    // (U ≥ 0: the bit patterns of non-negative floats order as the values)
    let key = |v: f64| std::cmp::Reverse(v.to_bits());
    heap.push((key(u[start.0][start.1]), start));
    while let Some((std::cmp::Reverse(bits), (j, i))) = heap.pop() {
        let c = f64::from_bits(bits);
        if std::mem::replace(&mut done[j][i], true) {
            continue;
        }
        if exit(j, i) {
            return c - u[start.0][start.1];
        }
        for (dj, di) in [(-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 0), (1, 1)] {
            let (jj, ii) = (j as i64 + dj, i as i64 + di);
            if (0..ny as i64).contains(&jj) && (0..nx as i64).contains(&ii) && !done[jj as usize][ii as usize] {
                let v = c.max(u[jj as usize][ii as usize]);
                heap.push((key(v), (jj as usize, ii as usize)));
            }
        }
    }
    f64::NAN
}

/// (pseudopotential, Floquet) frequencies of the modes, lowest first.
fn freqs(a: &Value) -> Vec<(f64, f64)> {
    a["modes"].as_array().unwrap().iter().map(|x| (x["freq_pseudo"].as_f64().unwrap(), x["freq_floquet"].as_f64().unwrap())).collect()
}

/// Mesh and solve an example with its own settings; returns the app and its drive.
fn solved(design: Design) -> (App, PathBuf, Value) {
    solved_in(copy_example(design))
}

fn solved_in(dir: PathBuf) -> (App, PathBuf, Value) {
    let mut app = App::new(&dir).unwrap();
    let st = cmd(&mut app, "startup", Value::Null);
    cmd(&mut app, "mesh", st["project"]["mesh_params"].clone());
    cmd(&mut app, "solve", st["project"]["solve_params"].clone());
    (app, dir, st["project"]["trap_params"].clone())
}

fn ideal_paul_trap_is_an_ideal_quadrupole(dir: PathBuf) {
    let (mut app, dir, drive) = solved_in(dir);
    let r0 = demo::PAUL_R0;
    // the potential of the ring at 1 V against the ideal hyperbolic trap's
    let pts = [[0.0, 0.0, 0.0], [0.2 * r0, 0.0, 0.0], [0.0, 0.0, 0.2 * r0], [0.1 * r0, -0.15 * r0, 0.12 * r0], [0.3 * r0, 0.0, -0.2 * r0]];
    let ev = cmd(&mut app, "eval", json!({ "points": pts, "voltages": { "ring": 1.0 } }));
    for (p, phi) in pts.iter().zip(ev["phi"].as_array().unwrap()) {
        let ideal = 0.5 + (p[0] * p[0] + p[1] * p[1] - 2.0 * p[2] * p[2]) / (2.0 * r0 * r0);
        assert!((phi.as_f64().unwrap() - ideal).abs() < 1e-3, "φ{p:?} = {phi}, ideal {ideal}");
    }

    let a = cmd(&mut app, "trap", drive.clone())["analysis"].clone();
    assert!(a["confined"].as_bool().unwrap(), "{a:#}");
    let eq: Vec<f64> = a["equilibrium"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
    assert!(eq.iter().all(|x| x.abs() < 1e-4 * r0), "equilibrium {eq:?}");
    assert!(a["rf_null_line"].is_null(), "a 3D trap has a point null");
    // Mathieu q along z, and the pseudopotential frequencies qΩ/(2√2) (z) and half that (r)
    let (m, w) = (39.96259 * AMU, 2.0 * PI * demo::PAUL_FREQ);
    let qz = 4.0 * E * demo::PAUL_V / (m * w * w * r0 * r0);
    let q: Vec<f64> = a["mathieu"].as_array().unwrap().iter().map(|x| x["q"].as_f64().unwrap()).collect();
    assert!((q[0].abs() - qz).abs() < 2e-3 * qz, "q {q:?}, ideal q_z {qz}");
    let f: Vec<f64> = a["modes"].as_array().unwrap().iter().map(|x| x["freq_pseudo"].as_f64().unwrap()).collect();
    let fz = qz * demo::PAUL_FREQ / (2.0 * 2f64.sqrt());
    assert!((f[2] - fz).abs() < 2e-3 * fz, "axial {} Hz, ideal {fz} Hz", f[2]);
    for fr in &f[..2] {
        assert!((fr - fz / 2.0).abs() < 2e-3 * fz, "radial {fr} Hz, ideal {} Hz", fz / 2.0);
    }
    // at q_z ≈ 0.49 the pseudopotential is 5 % low; the Floquet frequencies against the
    // exact solutions of Mathieu's equation (182.4 kHz axial, 87.5 kHz radial)
    let fl = freqs(&a);
    let (bz, br) = (mathieu_beta(0.0, qz) * demo::PAUL_FREQ / 2.0, mathieu_beta(0.0, qz / 2.0) * demo::PAUL_FREQ / 2.0);
    assert!((fl[2].1 - bz).abs() < 2e-3 * bz, "axial Floquet {} Hz, exact {bz} Hz", fl[2].1);
    for (_, fr) in &fl[..2] {
        assert!((fr - br).abs() < 2e-3 * br, "radial Floquet {fr} Hz, exact {br} Hz");
    }
    // depth: q_z V/16 = 6.12 eV, at the ring. The field within about a panel of the
    // surface is less accurate, which puts the solved depth a few percent low.
    let mut with_depth = drive.clone();
    with_depth["depth"] = json!(true);
    let r = cmd(&mut app, "trap", with_depth);
    let (d, ideal) = (r["depth_ev"].as_f64().unwrap(), qz * demo::PAUL_V / 16.0);
    assert_eq!(r["depth"]["exit"], "electrode");
    assert!(d < ideal && d > 0.93 * ideal, "depth {d} eV, ideal {ideal} eV");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The stretched trap of March's worked example: r₀ = 1.00 cm, z₀ = 0.783 cm, 757 V (0-p)
/// at 1.05 MHz, m/z 134. Published: q_z = 8eV/(m(r₀² + 2z₀²)Ω²) = 0.450 and
/// ω_z = β_zΩ/2 = 167 kHz with β_z ≈ q_z/√2.
fn march_trap_matches_the_paper() {
    let (mut app, dir, drive) = solved(Design::Paul);
    let a = cmd(&mut app, "trap", drive.clone())["analysis"].clone();
    assert!(a["confined"].as_bool().unwrap(), "{a:#}");
    let qz = a["mathieu"].as_array().unwrap().iter().find(|m| m["axis"][2].as_f64().unwrap().abs() > 0.9).unwrap()["q"].as_f64().unwrap();
    // (the paper's q_z formula is approximate for a stretched trap: within 1 %)
    assert!((qz - 0.450).abs() < 0.01 * 0.450, "q_z {qz}, paper 0.450");
    let f = freqs(&a);
    let (fz, fz_exact) = f[2];
    // the paper's lowest-order frequency, q_zΩ/(2√2): the pseudopotential one
    assert!((fz - 167e3).abs() < 0.01 * 167e3, "axial {fz} Hz, paper 167 kHz");
    // the exact frequency for the solved q, and for the paper's q (174.5 kHz)
    let exact = mathieu_beta(0.0, qz) * demo::MARCH_FREQ / 2.0;
    assert!((fz_exact - exact).abs() < 2e-3 * exact, "axial Floquet {fz_exact} Hz, Mathieu {exact} Hz");
    let exact_paper = mathieu_beta(0.0, 0.450) * demo::MARCH_FREQ / 2.0;
    assert!((fz_exact - exact_paper).abs() < 0.01 * exact_paper, "axial Floquet {fz_exact} Hz, Mathieu at q = 0.450: {exact_paper} Hz");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

fn surface_trap_matches_the_gapless_model_and_the_paper() {
    let (mut app, dir, drive) = solved(Design::Surface);
    let a = cmd(&mut app, "trap", drive.clone())["analysis"].clone();
    let v = |x: &Value| -> Vec<f64> { x.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect() };
    let null = v(&a["rf_null"]);

    // the RF null (in the cross-section) against the gapless plane's: the 10 µm gaps
    // and finite (5 mm) rails move it by about 1 %
    let rails = demo::gapless_rails();
    let g = demo::gapless_null(&rails, [0.0, demo::SURF_HEIGHT]);
    assert!((null[0] - g[0]).hypot(null[1] - g[1]) < 0.02 * g[1], "RF null {null:?}, gapless {g:?}");
    // and against the published ion height of about 240 µm
    assert!((null[1] - demo::SURF_HEIGHT).abs() < 0.06 * demo::SURF_HEIGHT, "null height {} µm", null[1] * 1e6);
    // the asymmetric rails tilt the RF principal axes (gapless: 16°)
    let h = 1e-6;
    let phi = |dx: f64, dy: f64| demo::gapless_phi(&rails, g[0] + dx, g[1] + dy);
    let (pxx, pyy) = ((phi(h, 0.0) + phi(-h, 0.0) - 2.0 * phi(0.0, 0.0)) / (h * h), (phi(0.0, h) + phi(0.0, -h) - 2.0 * phi(0.0, 0.0)) / (h * h));
    let pxy = (phi(h, h) - phi(h, -h) - phi(-h, h) + phi(-h, -h)) / (4.0 * h * h);
    let tilt_gapless = (0.5 * (2.0 * pxy).atan2(pxx - pyy)).to_degrees().abs();
    let tilt_gapless = tilt_gapless.min(90.0 - tilt_gapless);
    let radial = a["mathieu"].as_array().unwrap().iter().map(|x| v(&x["axis"])).find(|ax| ax[2].abs() < 0.1).unwrap();
    let tilt = (radial[0].abs().atan2(radial[1].abs())).to_degrees();
    let tilt = tilt.min(90.0 - tilt);
    assert!((tilt - tilt_gapless).abs() < 1.5, "RF axes tilted {tilt}°, gapless {tilt_gapless}°");
    // RF-only radial frequency against the gapless plane's (1.596 MHz pseudopotential):
    // the solved chip, with gaps and no ground plane around it, is ~4 % weaker
    let mut rf_only = drive.clone();
    for (_, u) in rf_only["dc"].as_object_mut().unwrap() {
        *u = json!(0.0);
    }
    let r = cmd(&mut app, "trap", rf_only)["analysis"].clone();
    let lam = {
        let tr = pxx + pyy;
        let det = pxx * pyy - pxy * pxy;
        (0.25 * tr * tr - det).sqrt() + 0.5 * tr.abs()
    };
    let (m, w) = (39.96259 * AMU, 2.0 * PI * demo::SURF_FREQ);
    let f_gapless = E * lam * demo::SURF_V / (2f64.sqrt() * m * w) / (2.0 * PI);
    for (fp, _) in &freqs(&r)[1..] {
        assert!(*fp < f_gapless && *fp > 0.94 * f_gapless, "RF-only radial {fp} Hz, gapless {f_gapless} Hz");
    }
    // RF-only depth in the cross-section (escape upward), against the gapless plane's on
    // the same grid (79 meV): ~9 % less, as the weaker confinement above
    let (xs, ys): (Vec<f64>, Vec<f64>) = ((0..57).map(|i| -700e-6 + 25e-6 * i as f64).collect(), (0..56).map(|j| 25e-6 + 25e-6 * j as f64).collect());
    let pts: Vec<[f64; 3]> = ys.iter().flat_map(|&y| xs.iter().map(move |&x| [x, y, 0.0])).collect();
    let g = rf_grad(&mut app, &drive, &pts);
    let u_bem: Vec<Vec<f64>> = g.chunks(xs.len()).map(|row| row.iter().map(|&e| pseudo_ev(e, demo::SURF_FREQ)).collect()).collect();
    let h = 1e-8;
    let u_gap: Vec<Vec<f64>> = ys
        .iter()
        .map(|&y| {
            xs.iter()
                .map(|&x| {
                    let gx = (demo::gapless_phi(&rails, x + h, y) - demo::gapless_phi(&rails, x - h, y)) / (2.0 * h) * demo::SURF_V;
                    let gy = (demo::gapless_phi(&rails, x, y + h) - demo::gapless_phi(&rails, x, y - h)) / (2.0 * h) * demo::SURF_V;
                    pseudo_ev([gx, gy, 0.0], demo::SURF_FREQ)
                })
                .collect()
        })
        .collect();
    let top = ys.len() - 1;
    let (d_bem, d_gap) = (flood_depth(&u_bem, |j, _| j == top), flood_depth(&u_gap, |j, _| j == top));
    assert!(d_bem < d_gap && d_bem > 0.85 * d_gap, "RF-only depth {d_bem} eV, gapless {d_gap} eV");
    // with the DC set (fitted to the paper's frequencies, so softer vertically than a set
    // chosen for depth) the ion's lowest way out is a saddle ~115 µm above it: ~38 meV
    let mut with_depth = drive.clone();
    with_depth["depth"] = json!(true);
    let r = cmd(&mut app, "trap", with_depth);
    let d = r["depth_ev"].as_f64().unwrap();
    assert!(d > 0.03 && d < 0.05, "depth {d} eV ({})", r["depth"]["exit"]);

    // the fitted DC set holds the ion on the null, with the published exact frequencies
    // (horizontal, vertical, axial) ≈ (1.2, 1.4, 0.4) MHz and motional axes tilted ~25°
    assert!(a["confined"].as_bool().unwrap(), "{a:#}");
    assert!(a["notes"].as_array().unwrap().is_empty(), "{:#}", a["notes"]);
    let eq = v(&a["equilibrium"]);
    assert!((eq[0] - null[0]).hypot(eq[1] - null[1]) < 1e-6, "ion {eq:?}, null {null:?}");
    let modes = a["modes"].as_array().unwrap();
    for (m, want) in modes.iter().zip([0.4e6, 1.2e6, 1.4e6]) {
        let f = m["freq_floquet"].as_f64().unwrap();
        assert!((f - want).abs() < 0.01 * want, "mode {} Hz, paper {want} Hz", f);
    }
    let ax = v(&modes[1]["axis"]);
    let mode_tilt = ax[1].abs().atan2(ax[0].abs()).to_degrees();
    assert!((mode_tilt - 25.0).abs() < 1.0, "horizontal mode tilted {mode_tilt}°, paper ≈ 25°");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The segmented four-rod trap of Miao et al. (arXiv:2201.03377) at its published
/// operating point (⁴⁰Ca⁺, 150 V on rods B2 and B4 at 1.961 MHz, 10 V on the end
/// segments): the paper's q_Ca = 0.124 and a ≈ 0, and the radial frequencies and depth
/// of four infinite rods (examples/reference_frequencies.py: 85.95 kHz pseudopotential,
/// 86.22 kHz exact with the endcaps' defocusing; saddle 3.811 eV at x = 9.41 mm).
fn segmented_trap_matches_the_paper_and_infinite_rods() {
    let (mut app, dir, drive) = solved(Design::Segmented);
    let a = cmd(&mut app, "trap", drive.clone())["analysis"].clone();
    assert!(a["confined"].as_bool().unwrap(), "{a:#}");
    let mathieu: Vec<(f64, f64, Vec<f64>)> = a["mathieu"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| (m["q"].as_f64().unwrap(), m["a"].as_f64().unwrap(), m["axis"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect()))
        .collect();
    for (q, a_, axis) in &mathieu {
        if axis[2].abs() < 0.1 {
            assert!((q.abs() - 0.124).abs() < 0.005 * 0.124 + 5e-4, "radial q {q}, paper 0.124");
        }
        assert!(a_.abs() < 1e-3, "a {a_}, paper ≈ 0");
    }
    let f = freqs(&a);
    for (fp, ff) in &f[1..] {
        assert!((fp - 85.95e3).abs() < 0.005 * 85.95e3, "radial {fp} Hz, infinite rods 85.95 kHz");
        assert!((ff - 86.22e3).abs() < 0.005 * 86.22e3, "radial Floquet {ff} Hz, infinite rods 86.22 kHz");
    }
    // the RF-only saddle between neighbouring rods, on the x axis (the depth itself is
    // lower: the pseudopotential at the rod faces is ~2.3 eV)
    let pts: Vec<[f64; 3]> = (0..=60).map(|i| [8.5e-3 + 0.03e-3 * i as f64, 0.0, 0.0]).collect();
    let saddle = rf_grad(&mut app, &drive, &pts).into_iter().map(|e| pseudo_ev(e, demo::SEG_FREQ)).fold(0.0, f64::max);
    assert!((saddle - 3.811).abs() < 0.03 * 3.811, "radial saddle {saddle} eV, infinite rods 3.811 eV");
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}
