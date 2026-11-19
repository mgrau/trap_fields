//! Mesh-quality and solve diagnostics for a TFM1 surface mesh:
//!   cargo run --release -p tf-bem --example diagnose -- mesh/surface_mesh.bin
use tf_bem::panel::{dot, norm, single_layer, sub};
use tf_bem::{solve, Conductor, Problem};

fn main() {
    let path = std::env::args().nth(1).expect("path to surface_mesh.bin");
    let b = std::fs::read(&path).unwrap();
    assert_eq!(&b[0..4], b"TFM1");
    let u32at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let f32at = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as f64;
    let (nn, nt) = (u32at(4) as usize, u32at(8) as usize);
    let mut o = 12;
    let nodes: Vec<f64> = (0..3 * nn).map(|i| f32at(o + 4 * i)).collect();
    o += 12 * nn;
    let tris: Vec<u32> = (0..3 * nt).map(|i| u32at(o + 4 * i)).collect();
    o += 12 * nt;
    let body: Vec<usize> = (0..nt).map(|i| u32at(o + 4 * i) as usize).collect();
    let nb = body.iter().max().unwrap() + 1;
    let conductors = (0..nb).map(|k| Conductor { name: format!("b{k}"), floating: false }).collect();
    let p = Problem::from_mesh(&nodes, &tris, body.clone(), conductors).unwrap();
    println!("{nt} panels, {nn} nodes, {nb} bodies");

    // quality: aspect = longest edge / (2·inradius·√3) (1 for equilateral)
    let mut worst: Vec<(f64, usize)> = p
        .panels
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let e = [norm(sub(q.v[1], q.v[0])), norm(sub(q.v[2], q.v[1])), norm(sub(q.v[0], q.v[2]))];
            let s = (e[0] + e[1] + e[2]) / 2.0;
            let inr = q.area / s;
            (e.iter().cloned().fold(0.0, f64::max) / (2.0 * 3f64.sqrt() * inr), i)
        })
        .collect();
    worst.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    println!("worst aspect ratios: {:?}", &worst[..5.min(worst.len())]);
    // panels whose centroid lies very close to another panel (overlaps / duplicates)
    let mut close = 0;
    for (i, a) in p.panels.iter().enumerate() {
        for (j, c) in p.panels.iter().enumerate() {
            if i != j {
                let d = sub(a.centroid, tf_bem::panel::closest_point(c, a.centroid));
                if dot(d, d).sqrt() < 1e-3 * a.radius.min(c.radius) && dot(a.normal, c.normal).abs() < 0.999 || (i < j && norm(sub(a.centroid, c.centroid)) < 1e-6 * a.radius) {
                    if close < 10 {
                        println!("  panel {i} (body {}) touches panel {j} (body {}) at its centroid", body[i], body[j]);
                    }
                    close += 1;
                }
            }
        }
    }
    println!("{close} centroid-on-other-panel incidences");

    let s = solve(&p, &|_| {}).unwrap();
    println!("{:?}", s.timings);
    // residual of the collocation equations, recomputed from scratch
    for (bi, u) in s.u.iter().enumerate() {
        let mut worst: f64 = 0.0;
        for (i, pi) in p.panels.iter().enumerate() {
            let v: f64 = p.panels.iter().zip(u).map(|(pj, uj)| uj * single_layer(pj, pi.centroid)).sum::<f64>() / (4.0 * std::f64::consts::PI);
            let target = if body[i] == s.basis[bi] { 1.0 } else { 0.0 };
            worst = worst.max((v - target).abs());
        }
        println!("basis {bi}: max residual {worst:.3e}, charge {:?}", s.charge[bi].iter().map(|q| q * 1e15).collect::<Vec<_>>());
    }
}
