//! Benchmark of the post-meshing pipeline on a TFM1 mesh, identical natively and in
//! WebAssembly:
//!
//!   tf-bench <mesh.bin> [--nodes mesh.nodes.f64] [--step file.step] [--threads N] [--lmax L]
//!
//! Bodies are electrodes in mesh order (trap: Blade 1–4, Endcap 1–2), driven two-phase:
//! RF ±200 V on the blade pairs, 10 V DC on the endcaps, ⁴⁰Ca⁺ at 30 MHz. Prints one JSON
//! object per step with its wall time, and the physics results for cross-checking.
use std::time::Instant;
use tf_bem::trap::{depth_estimate, energy_direct, inside_surface, Fields, Ion};
use tf_bem::{expand, moment_table, Conductor, Expansion, Problem};

#[path = "../../tf-geom/src/step.rs"]
#[allow(dead_code)]
mod step;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn report(step: &str, seconds: f64, extra: serde_json::Value) {
    let mut v = serde_json::json!({ "step": step, "seconds": (seconds * 1e4).round() / 1e4 });
    if let (Some(o), serde_json::Value::Object(e)) = (v.as_object_mut(), extra) {
        o.extend(e);
    }
    println!("{v}");
}

fn decode(bin: &[u8], nodes_f64: Option<Vec<u8>>) -> (Vec<f64>, Vec<u32>, Vec<u32>) {
    assert_eq!(&bin[0..4], b"TFM1", "not a TFM1 mesh");
    let u = |o: usize| u32::from_le_bytes(bin[o..o + 4].try_into().unwrap());
    let (nn, nt) = (u(4) as usize, u(8) as usize);
    let mut nodes: Vec<f64> = (0..3 * nn).map(|i| f32::from_le_bytes(bin[12 + 4 * i..16 + 4 * i].try_into().unwrap()) as f64).collect();
    let o = 12 + 12 * nn;
    let tris: Vec<u32> = (0..3 * nt).map(|i| u(o + 4 * i)).collect();
    let body: Vec<u32> = (0..nt).map(|i| u(o + 12 * nt + 4 * i)).collect();
    if let Some(raw) = nodes_f64.filter(|r| r.len() == 24 * nn) {
        nodes = raw.chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap())).collect();
    }
    (nodes, tris, body)
}

fn run(args: &[String]) {
    let lmax: usize = arg(args, "--lmax").map(|s| s.parse().unwrap()).unwrap_or(10);

    if let Some(path) = arg(args, "--step") {
        let text = std::fs::read_to_string(&path).expect("read STEP");
        let t = Instant::now();
        let info = step::parse_step(&text);
        report("parse_step_names", t.elapsed().as_secs_f64(), serde_json::json!({ "solids": info.solids.len() }));
    }

    let bin = std::fs::read(&args[1]).expect("read mesh");
    let (nodes, tris, body) = decode(&bin, arg(args, "--nodes").and_then(|p| std::fs::read(p).ok()));
    let nb = body.iter().map(|&b| b as usize + 1).max().unwrap();
    let conductors = (0..nb).map(|k| Conductor { name: format!("body{k}"), floating: false }).collect();
    let problem = Problem::from_mesh(&nodes, &tris, body.iter().map(|&b| b as usize).collect(), conductors).unwrap();
    let n = problem.panels.len();

    let t = Instant::now();
    let sol = tf_bem::solve(&problem, &|_| {}).unwrap();
    let total = t.elapsed().as_secs_f64();
    let tm = &sol.timings;
    report("bem_assemble", tm.assemble, serde_json::json!({ "panels": n, "matrix_gb": 8.0 * (n as f64).powi(2) / 1e9 }));
    report("bem_lu_factor", tm.factor, serde_json::json!({}));
    report("bem_solve_rhs", tm.solve, serde_json::json!({ "rhs": sol.u.len() }));
    report("bem_total", total, serde_json::json!({}));

    let t = Instant::now();
    let r_min = problem.distance_to_surface([0.0; 3]);
    let table = moment_table(&problem.panels, [0.0; 3], r_min, lmax);
    let exps: Vec<Expansion> = sol.u.iter().map(|u| expand(&table, u, [0.0; 3], r_min, lmax)).collect();
    report("expansions", t.elapsed().as_secs_f64(), serde_json::json!({ "lmax": lmax, "r_min_um": r_min * 1e6, "phi0_body0": exps[0].coeffs[0] }));

    // two-phase drive on the blades, DC on the endcaps
    let rf: Vec<f64> = (0..nb).map(|k| [200.0, -200.0, 200.0, -200.0].get(k).copied().unwrap_or(0.0)).collect();
    let dc: Vec<f64> = (0..nb).map(|k| if k >= 4 { 10.0 } else { 0.0 }).collect();
    let comb = |w: &[f64]| {
        let mut u = vec![0.0; n];
        for (b, uk) in sol.u.iter().enumerate() {
            u.iter_mut().zip(uk).for_each(|(a, x)| *a += w[b] * x);
        }
        (u, Expansion::combine(w.iter().copied().zip(&exps)).unwrap())
    };
    let (u_rf, e_rf) = comb(&rf);
    let (u_dc, e_dc) = comb(&dc);
    let ion = Ion { mass_amu: 39.96259, charge_e: 1.0 };
    let omega = 2.0 * std::f64::consts::PI * 30e6;
    let fields = Fields { rf: e_rf, dc: e_dc, omega, ion, r_valid: r_min };
    let reps = 20;
    let t = Instant::now();
    let mut a = fields.analyze();
    for _ in 1..reps {
        a = fields.analyze();
    }
    let freqs: Vec<f64> = a.modes.iter().map(|m| m.freq_pseudo.unwrap_or(f64::NAN) / 1e3).collect();
    report("trap_analysis", t.elapsed().as_secs_f64() / reps as f64, serde_json::json!({ "freqs_khz": freqs, "confined": a.confined }));

    let t = Instant::now();
    let d = depth_estimate(&problem.panels, &u_rf, &u_dc, ion, omega, a.equilibrium, a.energy_eq, 10.0 * r_min);
    report("trap_depth", t.elapsed().as_secs_f64(), serde_json::json!({ "depth_ev": d.depth / 1.602176634e-19, "rays": d.n_rays }));

    let ns = 121;
    let half = r_min;
    let pts: Vec<[f64; 3]> = (0..ns * ns)
        .map(|k| {
            let (i, j) = (k % ns, k / ns);
            [0.0, half * (2.0 * i as f64 / (ns - 1) as f64 - 1.0), half * (2.0 * j as f64 / (ns - 1) as f64 - 1.0)]
        })
        .collect();
    let t = Instant::now();
    let inside = {
        use rayon::prelude::*;
        pts.par_iter().filter(|&&p| inside_surface(&problem.panels, p)).count()
    };
    let u = energy_direct(&problem.panels, &u_rf, &u_dc, ion, omega, &pts);
    report("slice_121x121", t.elapsed().as_secs_f64(), serde_json::json!({ "inside": inside, "u_center_ev": u[ns * ns / 2] / 1.602176634e-19 }));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: tf-bench <mesh.bin> [--nodes f] [--step f] [--threads N] [--lmax L]");
        std::process::exit(2);
    }
    let threads: usize = arg(&args, "--threads").map(|s| s.parse().unwrap()).unwrap_or(0);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().ok();
    let used = pool.as_ref().map(|p| p.current_num_threads()).unwrap_or(1);
    if used == 1 {
        faer::set_global_parallelism(faer::Par::Seq);
    }
    report("environment", 0.0, serde_json::json!({ "arch": std::env::consts::ARCH, "os": std::env::consts::OS, "threads": used }));
    match pool {
        Some(p) => p.install(|| run(&args)),
        None => run(&args),
    }
}
