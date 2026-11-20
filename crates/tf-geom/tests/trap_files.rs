//! Integration test against the trap STEP files in the repository root (skipped if absent).
//! gmsh is a global singleton, so everything runs inside one test.
use std::path::PathBuf;
use tf_geom::{defeature, load_step, mesh_bem, metrics, tessellate, MeshParams, Rigid, TransformOp};
use tf_gmsh::Gmsh;

fn file(name: &str) -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(name);
    p.exists().then_some(p)
}

#[test]
fn trap_step_files() {
    let mut g = Gmsh::initialize().unwrap();

    if let Some(p) = file("trap_electrodes.step") {
        let mut m = load_step(&mut g, &p).unwrap();
        let names: Vec<&str> = m.bodies.iter().map(|b| b.name.as_str()).collect();
        println!("bodies: {names:?}  warnings: {:?}  load {:.2}s", m.warnings, m.load_seconds);
        assert_eq!(names, ["Blade 1", "Blade 2", "Blade 3", "Blade 4", "Endcap 1", "Endcap 2"]);
        assert!((m.bodies[0].volume * 1e9 - 1489.379).abs() < 0.01, "blade volume in mm^3");

        let t = tessellate(&mut g, &mut m, 0.6e-3).unwrap();
        println!(
            "display tess: {} tris, {} edge segs, {} points, failed {:?}, {:.2}s",
            t.tri_body.len(), t.edge_body.len(), t.point_body.len(), t.failed_faces, t.seconds
        );
        assert!(t.failed_faces.is_empty());
        let near = metrics::closest_per_group([0.0; 3], &t.positions, &t.triangles, &t.tri_body, m.bodies.len());
        for (b, n) in m.bodies.iter().zip(&near) {
            println!("  {:<9} closest to origin {:8.1} um", b.name, n.unwrap().0 * 1e6);
        }
        let blade = near[0].unwrap().0;
        assert!((blade - 750.6e-6).abs() < 5e-6, "blade tip distance {blade}");

        // rotate the trap so its axis (x) points along z, then mesh around the origin
        let frame = Rigid::from_ops(&[TransformOp::Rotate { axis: [0., 1., 0.], angle_deg: -90., center: [0.; 3] }]);
        let params = MeshParams { center: [0.0; 3], h_min: 30e-6, h_max: 2e-3, dist_min: 0.75e-3, dist_max: 7.5e-3, min_circle_points: 16 };
        let mesh = mesh_bem(&mut g, &mut m, &params, &frame).unwrap();
        println!("BEM mesh: {:?}", mesh.stats);
        assert!(mesh.stats.failed_faces.is_empty());
        assert!(mesh.stats.n_triangles > 10_000);
        // endcaps now lie along z
        let zmax = mesh.nodes.chunks_exact(3).map(|p| p[2]).fold(f64::MIN, f64::max);
        assert!((zmax - 28.216e-3).abs() < 0.01e-3, "zmax {zmax}");
    }

    // same trap with all threads removed (filled holes): identical tips and endcaps, few faces
    if let Some(p) = file("trap_electrodes_nothread.step") {
        let mut m = load_step(&mut g, &p).unwrap();
        let names: Vec<&str> = m.bodies.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["Blade 1", "Blade 2", "Blade 3", "Blade 4", "Endcap 1", "Endcap 2"]);
        assert!(m.bodies[..4].iter().all(|b| b.n_faces == 37), "blades without threads");
        assert!((m.bodies[4].volume * 1e9 - 449.984).abs() < 0.01, "endcap volume in mm^3");
        let t = tessellate(&mut g, &mut m, 0.6e-3).unwrap();
        assert!(t.failed_faces.is_empty());
        let near = metrics::closest_per_group([0.0; 3], &t.positions, &t.triangles, &t.tri_body, m.bodies.len());
        let blade = near[0].unwrap().0;
        assert!((blade - 750.0e-6).abs() < 5e-6, "blade tip distance {blade}");
        let params = MeshParams { center: [0.0; 3], h_min: 30e-6, h_max: 2.9e-3, dist_min: 0.75e-3, dist_max: 7.5e-3, min_circle_points: 16 };
        let mesh = mesh_bem(&mut g, &mut m, &params, &Rigid::default()).unwrap();
        println!("no-thread BEM mesh: {:?}", mesh.stats);
        assert!(mesh.stats.failed_faces.is_empty() && mesh.stats.n_triangles > 5_000);
        assert!(mesh.stats.defective_faces.is_empty(), "defects {:?}", mesh.stats.defective_faces);
        // these sizes used to give folded/duplicate triangles on the 1 mm hole walls
        let fine = MeshParams { h_min: 15e-6, h_max: 1.5e-3, min_circle_points: 24, ..params };
        let mesh = mesh_bem(&mut g, &mut m, &fine, &Rigid::default()).unwrap();
        println!("no-thread fine BEM mesh: {:?}", mesh.stats);
        assert!(mesh.stats.defective_faces.is_empty(), "defects {:?}", mesh.stats.defective_faces);

        // remove a screw hole of blade 1: pick its largest cylindrical face, grow it to the
        // whole hole, remove it; a reload replays the stored removal to the same model
        let b = &m.bodies[0];
        let seed = m.faces.iter().filter(|f| f.body == 0 && f.kind == "Cylinder").max_by(|a, b| a.area.total_cmp(&b.area)).map(|f| f.tag);
        let seed = seed.expect("blade 1 has a cylindrical face");
        let feature = defeature::grow_feature(&g, &m, seed).unwrap();
        println!("hole of {}: seed {seed}, feature faces {feature:?}", b.name);
        assert!(!feature.is_empty() && feature.len() < b.n_faces / 2);
        let r = defeature::describe(&g, &m, &feature).unwrap();
        let (n0, v0) = (m.bodies[0].n_faces, m.bodies[0].volume);
        defeature::apply(&mut g, &mut m, &r).unwrap();
        let (n1, v1) = (m.bodies[0].n_faces, m.bodies[0].volume);
        println!("faces {n0} -> {n1}, volume {:.3} -> {:.3} mm^3", v0 * 1e9, v1 * 1e9);
        assert!(n1 < n0 && v1 > v0, "the hole is filled");
        assert!(m.faces.iter().all(|f| m.bodies[f.body].faces.contains(&f.tag)));
        let t = tessellate(&mut g, &mut m, 0.6e-3).unwrap();
        assert!(t.failed_faces.is_empty());
        let mut m2 = load_step(&mut g, &p).unwrap();
        defeature::apply(&mut g, &mut m2, &r).unwrap();
        assert_eq!(m2.bodies[0].n_faces, n1);
        assert!((m2.bodies[0].volume - v1).abs() < 1e-15);
        // a face that is gone cannot be removed again
        assert!(defeature::apply(&mut g, &mut m2, &r).is_err());
    }

    if let Some(p) = file("trap_electrodes_threaded.step") {
        let mut m = load_step(&mut g, &p).unwrap();
        let t = tessellate(&mut g, &mut m, 0.6e-3).unwrap();
        println!("threaded file: {} tris, {} failed faces, {:.1}s", t.tri_body.len(), t.failed_faces.len(), t.seconds);
        assert!(!t.failed_faces.is_empty() && t.tri_body.len() > 10_000);
    }
}
