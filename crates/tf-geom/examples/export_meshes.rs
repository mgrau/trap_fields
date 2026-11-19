//! Run the native geometry pipeline (STEP load, display tessellation, graded BEM mesh)
//! and write the meshes for the benchmark at several refinements:
//!
//!   cargo run --release -p tf-geom --example export_meshes -- <file.step> <out_dir> [scale ...]
//!
//! Sizes are the GUI defaults for the trap (h_min 30 µm, h_max 2.9 mm, graded between
//! 0.75 and 7.5 mm from the origin) multiplied by each scale (default: 2.0 1.4 1.0 0.95),
//! or explicit "h_min,h_max,dist_min,dist_max" (m).
//! Writes <out_dir>/mesh_<n>.bin (TFM1) and .nodes.f64, and prints step timings as JSON.
use std::path::PathBuf;
use std::time::Instant;
use tf_geom::{load_step, mesh_bem, tessellate, MeshParams, Rigid};
use tf_gmsh::Gmsh;

fn tfm1(m: &tf_geom::SurfaceMesh) -> Vec<u8> {
    let (nn, nt) = (m.nodes.len() / 3, m.tri_body.len());
    let mut b = Vec::with_capacity(12 + 4 * (3 * nn + 5 * nt));
    b.extend_from_slice(b"TFM1");
    b.extend_from_slice(&(nn as u32).to_le_bytes());
    b.extend_from_slice(&(nt as u32).to_le_bytes());
    m.nodes.iter().for_each(|x| b.extend_from_slice(&(*x as f32).to_le_bytes()));
    for v in [&m.triangles, &m.tri_body, &m.tri_face] {
        v.iter().for_each(|x| b.extend_from_slice(&x.to_le_bytes()));
    }
    b
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let step = PathBuf::from(&args[1]);
    let out = PathBuf::from(&args[2]);
    // each spec is a scale factor, or explicit "h_min,h_max,dist_min,dist_max" in meters
    let specs: Vec<String> = if args.len() > 3 { args[3..].to_vec() } else { ["2.0", "1.4", "1.0", "0.95"].map(String::from).to_vec() };
    std::fs::create_dir_all(&out).unwrap();
    let mut g = Gmsh::initialize().unwrap();

    let t0 = Instant::now();
    let mut model = load_step(&mut g, &step).unwrap();
    let t_load = t0.elapsed().as_secs_f64();
    let t1 = Instant::now();
    let bb = g.bounding_box(-1, -1).unwrap();
    let diag = ((0..3).map(|k| (bb[k + 3] - bb[k]).powi(2)).sum::<f64>()).sqrt();
    let tess = tessellate(&mut g, &mut model, diag / 100.0).unwrap();
    let t_tess = t1.elapsed().as_secs_f64();
    println!("{{\"step\": \"load\", \"seconds\": {t_load:.4}, \"bodies\": {}}}", model.bodies.len());
    println!("{{\"step\": \"tessellate\", \"seconds\": {t_tess:.4}, \"triangles\": {}}}", tess.tri_body.len());

    let base = MeshParams { center: [0.0; 3], h_min: 30e-6, h_max: 2.9e-3, dist_min: 0.75e-3, dist_max: 7.5e-3, min_circle_points: 16 };
    for spec in specs {
        let v: Vec<f64> = spec.split(',').map(|x| x.parse().unwrap()).collect();
        let (s, p) = match v[..] {
            [s] => (s, MeshParams { h_min: base.h_min * s, h_max: base.h_max * s, ..base.clone() }),
            [a, b, c, d] => (0.0, MeshParams { h_min: a, h_max: b, dist_min: c, dist_max: d, ..base.clone() }),
            _ => panic!("bad mesh spec {spec}"),
        };
        let t = Instant::now();
        let m = mesh_bem(&mut g, &mut model, &p, &Rigid::default()).unwrap();
        let secs = t.elapsed().as_secs_f64();
        let n = m.tri_body.len();
        assert!(m.stats.defective_faces.is_empty(), "defective mesh at scale {s}");
        std::fs::write(out.join(format!("mesh_{n}.bin")), tfm1(&m)).unwrap();
        std::fs::write(out.join(format!("mesh_{n}.nodes.f64")), m.nodes.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
        println!("{{\"step\": \"mesh\", \"scale\": {s}, \"seconds\": {secs:.4}, \"triangles\": {n}, \"file\": \"mesh_{n}.bin\"}}");
    }
}
