//! The whole pipeline on a trap generated here (so it runs without CAD files, e.g. in
//! CI): the demo linear Paul trap of four rods and two needle endcaps, written as STEP by
//! gmsh, then loaded, meshed, solved and analyzed through the app's commands. Also checks
//! the electrode-displacement sensitivities against explicitly displaced solves.

use serde_json::{json, Value};
use std::path::PathBuf;
use tf_app::{demo, App, Event, Reply};

fn cmd(app: &mut App, name: &str, args: Value) -> Value {
    let sink = |_e: Event| {};
    match app.command(name, args, None, &sink) {
        Ok(Reply::Json(v)) => v,
        Ok(Reply::Binary(_)) => panic!("{name}: binary reply"),
        Err(e) => panic!("{name}: {} ({})", e.message, e.status),
    }
}

fn project_dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("trapfields-pipeline-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn v3(v: &Value) -> [f64; 3] {
    [0, 1, 2].map(|i| v[i].as_f64().unwrap())
}

#[test]
fn generated_trap_end_to_end() {
    let dir = project_dir();
    demo::create(&dir).unwrap();
    let mut app = App::new(&dir).unwrap();
    let st = cmd(&mut app, "startup", Value::Null);
    let bodies = st["bodies"].as_array().unwrap().clone();
    assert_eq!(bodies.len(), 6, "{st:#}");
    // the demo labels bodies by creation order: check they sit where their names say
    for b in &bodies {
        let bb: Vec<f64> = b["bbox"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        let c = [0.5 * (bb[0] + bb[3]), 0.5 * (bb[1] + bb[4]), 0.5 * (bb[2] + bb[5])];
        let e = b["label"]["electrode"].as_str().unwrap();
        let expect = if c[0].hypot(c[1]) < 1e-4 {
            (if c[2] > 0.0 { "EC+" } else { "EC-" }).to_string()
        } else {
            format!("rod {}{}", if c[0] > 0.0 { '+' } else { '-' }, if c[1] > 0.0 { '+' } else { '-' })
        };
        assert_eq!(e, expect, "{} at {c:?}", b["name"]);
    }

    let st = cmd(&mut app, "mesh", st["project"]["mesh_params"].clone());
    let panels = st["mesh"]["stats"]["n_triangles"].as_u64().unwrap();
    assert!((1000..20_000).contains(&panels), "{panels} panels");
    // the accuracy the tolerance study uses, so that explicit moves compare like with like
    let solve = json!({ "lmax": 8, "method": "compressed", "settings": { "aca_eps": 1e-8, "gmres_tol": 1e-10, "eta": 2.0, "leaf_size": 64 } });
    cmd(&mut app, "solve", solve.clone());

    let drive = serde_json::to_value(demo::drive()).unwrap();
    let t = cmd(&mut app, "trap", drive.clone());
    let a = t["analysis"].clone();
    assert!(a["confined"].as_bool().unwrap(), "{a:#}");
    let eq0 = v3(&a["equilibrium"]);
    // (the mesh is not exactly symmetric, and gmsh does not mesh it the same way every
    // run: the weak axial confinement puts the ion up to a few hundred nm off center)
    assert!(eq0.iter().all(|x| x.abs() < 1e-6), "equilibrium {eq0:?}");
    let f: Vec<f64> = a["modes"].as_array().unwrap().iter().map(|m| m["freq_pseudo"].as_f64().unwrap()).collect();
    // axial lowest, the two radial modes degenerate by symmetry
    assert!(f[0] < f[1] && (f[2] - f[1]).abs() < 0.01 * f[1], "{f:?}");
    assert!(a["rf_null_line"].is_array(), "a linear trap has a null line");

    // sensitivities to one rod, both ways
    let rod = "rod ++";
    cmd(&mut app, "tolerance", json!({ "electrodes": [rod], "translate_step": 2e-6, "rotate_step": 1e-3, "central": true }));
    let s = cmd(&mut app, "tolerance_eval", json!({ "params": drive, "sigma": { "translate": 5e-6, "rotate": 1e-3 } }));
    let quantities = s["quantities"].as_array().unwrap();
    let params = s["params"].as_array().unwrap();
    assert_eq!(params.len(), 6);
    let qi = |key: &str| quantities.iter().position(|q| q["key"] == key).unwrap();
    let pi = |label: &str| params.iter().position(|p| p["label"] == label).unwrap();
    let d1 = |p: usize, q: usize| s["d1"][p][q].as_f64().unwrap_or(f64::NAN);
    let (px, ex, ey) = (pi("x"), qi("eq_x"), qi("eq_y"));
    assert!(d1(px, ex).abs() > 1e-3, "the ion moves with the rod: {}", d1(px, ex));
    // the budget: the ion position's spread is the quadrature sum over the rod's six
    // degrees of freedom
    let b = &s["budget"]["rows"][ex];
    let expect = (0..6).map(|p| (d1(p, ex) * if p < 3 { 5e-6 } else { 1e-3 }).powi(2)).sum::<f64>().sqrt();
    assert!((b["sigma"].as_f64().unwrap() - expect).abs() < 1e-9 * expect, "{b}");

    // linear prediction against an explicit 4 µm move of the rod along x
    let step = 4e-6;
    cmd(&mut app, "moves", json!({ rod: { "translate": [step, 0.0, 0.0] } }));
    assert!(cmd(&mut app, "state", Value::Null)["solve"]["stale"].as_bool().unwrap(), "a move makes the solution stale");
    cmd(&mut app, "solve", solve.clone());
    let moved = cmd(&mut app, "trap", drive.clone());
    let eq1 = v3(&moved["analysis"]["equilibrium"]);
    for (q, c) in [(ex, 0), (ey, 1)] {
        let predicted = d1(px, q) * step;
        let actual = eq1[c] - eq0[c];
        assert!((actual - predicted).abs() <= 0.01 * predicted.abs() + 1e-10, "ion {c}: moved {actual:e} m, predicted {predicted:e} m");
    }
    // excess micromotion: the RF field at the ion, a small difference of the ion's and
    // the null's shifts
    let (e0, e1) = (v3(&a["rf_field_at_eq"]), v3(&moved["analysis"]["rf_field_at_eq"]));
    let predicted = d1(px, qi("erf_x")) * step;
    assert!((e1[0] - e0[0] - predicted).abs() <= 0.03 * predicted.abs() + 1e-3, "RF field at ion changed by {} V/m, predicted {predicted} V/m", e1[0] - e0[0]);

    // a rotation about the rod's own center: tilting it about y
    let (pty, fz) = (pi("θy"), qi("f1"));
    let angle = 2e-3;
    cmd(&mut app, "moves", json!({ rod: { "rotate": [0.0, angle, 0.0] } }));
    cmd(&mut app, "solve", solve.clone());
    let tilted = cmd(&mut app, "trap", drive.clone());
    let eq2 = v3(&tilted["analysis"]["equilibrium"]);
    let predicted = d1(pty, ex) * angle;
    // tilting about the center leaves the rod where it was at z = 0: little happens at the ion
    assert!((eq2[0] - eq0[0] - predicted).abs() <= 0.1 * predicted.abs() + 2e-9, "tilt: ion x moved {:e}, predicted {predicted:e}", eq2[0] - eq0[0]);
    let f2 = tilted["analysis"]["modes"][0]["freq_pseudo"].as_f64().unwrap();
    let d2 = s["d2"][pty][fz].as_f64().unwrap();
    let predicted = d1(pty, fz) * angle + 0.5 * d2 * angle * angle;
    assert!((f2 - f[0] - predicted).abs() <= 0.1 * predicted.abs() + 0.5, "axial frequency shift {} Hz, predicted {predicted} Hz", f2 - f[0]);

    // the moves and the solution come back after a restart
    drop(app);
    let mut app = App::new(&dir).unwrap();
    let st = cmd(&mut app, "startup", Value::Null);
    assert!(st["solve"].is_object() && !st["solve"]["stale"].as_bool().unwrap(), "{:#}", st["solve"]);
    assert!(st["project"]["moves"][rod].is_object());
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}
