//! Graded surface (BEM) mesh: fine near the trap center, coarse far away.

use crate::model::{mesh_surfaces, reset_mesh, GeomError, Model, Result};
use crate::transform::Rigid;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;
use tf_gmsh::{Gmsh, ELEMENT_TRIANGLE};

/// All lengths in meters; `center` is in the transformed (trap) frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshParams {
    pub center: [f64; 3],
    /// Element size within `dist_min` of the center.
    pub h_min: f64,
    /// Element size beyond `dist_max`.
    pub h_max: f64,
    pub dist_min: f64,
    pub dist_max: f64,
    #[serde(default = "default_circle_points")]
    pub min_circle_points: u32,
}

fn default_circle_points() -> u32 {
    16
}

impl MeshParams {
    pub fn validate(&self) -> Result<()> {
        let ok = self.h_min > 0.0
            && self.h_max >= self.h_min
            && self.dist_min >= 0.0
            && self.dist_max > self.dist_min
            && self.center.iter().all(|v| v.is_finite());
        if ok {
            Ok(())
        } else {
            Err(GeomError::Invalid(
                "Mesh parameters must satisfy 0 < h_min <= h_max and 0 <= dist_min < dist_max.".into(),
            ))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshStats {
    pub n_nodes: usize,
    pub n_triangles: usize,
    pub triangles_per_body: Vec<usize>,
    pub edge_min: f64,
    pub edge_max: f64,
    pub edge_mean: f64,
    pub area_total: f64,
    pub failed_faces: Vec<i32>,
    /// Faces whose mesh is still folded or has duplicate triangles after repair; a BEM
    /// solve on such a mesh is meaningless.
    #[serde(default)]
    pub defective_faces: Vec<i32>,
    /// Faces whose panel size was capped by their curvature.
    #[serde(default)]
    pub curvature_capped_faces: usize,
    /// Remeshing rounds needed to remove defects.
    #[serde(default)]
    pub repair_rounds: usize,
    pub seconds: f64,
}

/// Conforming surface mesh in the transformed frame (m).
#[derive(Debug, Clone)]
pub struct SurfaceMesh {
    pub nodes: Vec<f64>,
    pub triangles: Vec<u32>,
    pub tri_body: Vec<u32>,
    pub tri_face: Vec<u32>,
    pub params: MeshParams,
    pub stats: MeshStats,
}

/// Points per full circle used to cap the panel size on curved faces. This is for
/// robustness, not accuracy: with panels much larger than the radius of curvature gmsh
/// folds or duplicates triangles on such faces (seen on 1 mm holes through the blades).
/// Accuracy near the trap center comes from the distance grading.
pub const ROBUST_CIRCLE_POINTS: f64 = 6.0;

/// Panel-size caps from surface curvature: 2π R / ROBUST_CIRCLE_POINTS on curved faces
/// (and, through IncludeBoundary, their boundary curves). gmsh's own
/// Mesh.MeshSizeFromCurvature would do this but can stall on real CAD models.
fn curvature_caps(g: &Gmsh, model: &Model, params: &MeshParams) -> Result<HashMap<i32, f64>> {
    let mut caps = HashMap::new();
    let n = 7;
    for f in &model.faces {
        if g.entity_type(2, f.tag)? == "Plane" {
            continue;
        }
        let (lo, hi) = g.parametrization_bounds(2, f.tag)?;
        if lo.len() != 2 || hi.len() != 2 {
            continue;
        }
        let bb = g.bounding_box(2, f.tag)?;
        let pad = 1e-6 + 1e-3 * (0..3).map(|k| bb[k + 3] - bb[k]).fold(0.0, f64::max);
        let mut uv = Vec::with_capacity(2 * n * n);
        for i in 0..n {
            for j in 0..n {
                uv.push(lo[0] + (hi[0] - lo[0]) * (i as f64 + 0.5) / n as f64);
                uv.push(lo[1] + (hi[1] - lo[1]) * (j as f64 + 0.5) / n as f64);
            }
        }
        let xyz = g.value(2, f.tag, &uv)?;
        let kappa = g.curvature(2, f.tag, &uv)?;
        // ignore samples of the underlying surface outside the trimmed face (e.g. near a cone apex)
        let inside = |i: usize| (0..3).all(|k| xyz[3 * i + k] >= bb[k] - pad && xyz[3 * i + k] <= bb[k + 3] + pad);
        let kmax = (0..kappa.len()).filter(|&i| inside(i) && kappa[i].is_finite()).map(|i| kappa[i].abs()).fold(0.0, f64::max);
        if kmax > 0.0 {
            let cap = (2.0 * std::f64::consts::PI / (kmax * ROBUST_CIRCLE_POINTS)).max(params.h_min);
            if cap < params.h_max {
                caps.insert(f.tag, cap);
            }
        }
    }
    Ok(caps)
}

/// Background size field: distance grading from the center, limited by per-face caps.
fn size_field(g: &mut Gmsh, model: &mut Model, params: &MeshParams, center: [f64; 3], caps: &HashMap<i32, f64>) -> Result<()> {
    let [x0, y0, z0] = center;
    let dist = g.field_add("MathEval")?;
    model.fields.push(dist);
    g.field_set_string(dist, "F", &format!("Sqrt((x-({x0:e}))^2+(y-({y0:e}))^2+(z-({z0:e}))^2)"))?;
    let th = g.field_add("Threshold")?;
    model.fields.push(th);
    g.field_set_number(th, "InField", dist as f64)?;
    g.field_set_number(th, "SizeMin", params.h_min)?;
    g.field_set_number(th, "SizeMax", params.h_max)?;
    g.field_set_number(th, "DistMin", params.dist_min)?;
    g.field_set_number(th, "DistMax", params.dist_max)?;
    // one Constant field per cap value (binned to 2^(1/4) steps)
    let mut bins: HashMap<i32, (f64, Vec<f64>)> = HashMap::new();
    for (&tag, &cap) in caps {
        let k = (cap.log2() * 4.0).floor() as i32;
        let e = bins.entry(k).or_insert((f64::INFINITY, vec![]));
        e.0 = e.0.min(cap);
        e.1.push(tag as f64);
    }
    let mut fields = vec![th as f64];
    for (cap, tags) in bins.values() {
        let c = g.field_add("Constant")?;
        model.fields.push(c);
        g.field_set_number(c, "VIn", *cap)?;
        g.field_set_number(c, "VOut", 1e22)?;
        g.field_set_number(c, "IncludeBoundary", 1.0)?;
        g.field_set_numbers(c, "SurfacesList", tags)?;
        fields.push(c as f64);
    }
    let top = if fields.len() > 1 {
        let m = g.field_add("Min")?;
        model.fields.push(m);
        g.field_set_numbers(m, "FieldsList", &fields)?;
        m
    } else {
        th
    };
    g.field_set_as_background(top)?;
    // gmsh clamps the field to MeshSizeMin: let repair caps below h_min take effect
    let floor = caps.values().copied().fold(params.h_min, f64::min);
    g.set_number("Mesh.MeshSizeMin", floor)?;
    g.set_number("Mesh.MeshSizeMax", params.h_max)?;
    g.set_number("Mesh.MeshSizeFromPoints", 0.0)?;
    g.set_number("Mesh.MeshSizeExtendFromBoundary", 0.0)?;
    g.set_number("Mesh.MinimumCirclePoints", params.min_circle_points as f64)?;
    Ok(())
}

/// Per-face mesh defects: duplicate triangles, edges shared by more than two triangles,
/// and folds (neighbors on one CAD face with nearly opposite normals).
fn defective_faces(g: &Gmsh, model: &Model) -> Result<Vec<(i32, f64)>> {
    let mut bad = Vec::new();
    for f in &model.faces {
        let (_, conn) = g.mesh_elements_by_type(ELEMENT_TRIANGLE, f.tag)?;
        if conn.is_empty() {
            continue;
        }
        let (tags, xyz) = g.mesh_nodes(2, f.tag, true)?;
        let pos: HashMap<usize, [f64; 3]> = tags.iter().enumerate().map(|(i, &t)| (t, [xyz[3 * i], xyz[3 * i + 1], xyz[3 * i + 2]])).collect();
        let mut seen = std::collections::HashSet::new();
        let mut edges: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
        let mut normals = Vec::with_capacity(conn.len() / 3);
        let mut defect = false;
        let mut esum = 0.0;
        for (t, tri) in conn.chunks_exact(3).enumerate() {
            let mut k = [tri[0], tri[1], tri[2]];
            k.sort_unstable();
            defect |= !seen.insert(k);
            let (Some(a), Some(b), Some(c)) = (pos.get(&tri[0]), pos.get(&tri[1]), pos.get(&tri[2])) else {
                // keep normals indexed by triangle
                normals.push([0.0; 3]);
                defect = true;
                continue;
            };
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            defect |= !(l > 0.0);
            normals.push(if l > 0.0 { [n[0] / l, n[1] / l, n[2] / l] } else { [0.0; 3] });
            esum += (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt();
            for j in 0..3 {
                let (p, q) = (tri[j], tri[(j + 1) % 3]);
                edges.entry((p.min(q), p.max(q))).or_default().push(t);
            }
        }
        for ts in edges.values() {
            if ts.len() > 2 {
                defect = true;
            } else if ts.len() == 2 {
                let (a, b) = (normals[ts[0]], normals[ts[1]]);
                defect |= a[0] * b[0] + a[1] * b[1] + a[2] * b[2] < -0.5;
            }
        }
        if defect {
            bad.push((f.tag, esum / normals.len().max(1) as f64));
        }
    }
    Ok(bad)
}

pub fn mesh_bem(g: &mut Gmsh, model: &mut Model, params: &MeshParams, frame: &Rigid) -> Result<SurfaceMesh> {
    params.validate()?;
    let t0 = Instant::now();
    let center = frame.inverse().apply(params.center);
    let mut caps = curvature_caps(g, model, params)?;
    let curvature_capped_faces = caps.len();
    let mut failed_faces: Vec<i32>;
    let mut defects;
    let mut repair_rounds = 0;
    // faces meshed in the first pass: if a repair round loses one, that is a defect too
    let mut first_failed: Option<Vec<i32>> = None;
    loop {
        reset_mesh(g, model)?;
        size_field(g, model, params, center, &caps)?;
        failed_faces = mesh_surfaces(g, model)?;
        defects = defective_faces(g, model)?;
        let first = first_failed.get_or_insert_with(|| failed_faces.clone());
        for &f in &failed_faces {
            if !first.contains(&f) && !defects.iter().any(|d| d.0 == f) {
                defects.push((f, params.h_min));
            }
        }
        if defects.is_empty() || repair_rounds == 4 {
            break;
        }
        // remesh defective faces with half their current panel size (at least h_min/4);
        // stop when that changes nothing
        repair_rounds += 1;
        let mut changed = false;
        for &(tag, mean_edge) in &defects {
            let old = caps.get(&tag).copied().unwrap_or(f64::INFINITY);
            let cap = (old.min(mean_edge) * 0.5).max(params.h_min * 0.25);
            if cap < old * 0.999 {
                caps.insert(tag, cap);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let defective_faces: Vec<i32> = defects.iter().map(|d| d.0).collect();

    let mut nodes = Vec::new();
    let mut index: HashMap<usize, u32> = HashMap::new();
    let (mut triangles, mut tri_body, mut tri_face) = (Vec::new(), Vec::new(), Vec::new());
    for f in &model.faces {
        let (_, conn) = g.mesh_elements_by_type(ELEMENT_TRIANGLE, f.tag)?;
        if conn.is_empty() {
            continue;
        }
        let (tags, xyz) = g.mesh_nodes(2, f.tag, true)?;
        for (i, &n) in tags.iter().enumerate() {
            index.entry(n).or_insert_with(|| {
                let p = frame.apply([xyz[3 * i], xyz[3 * i + 1], xyz[3 * i + 2]]);
                nodes.extend_from_slice(&p);
                (nodes.len() / 3 - 1) as u32
            });
        }
        for tri in conn.chunks_exact(3) {
            let (Some(&a), Some(&b), Some(&c)) = (index.get(&tri[0]), index.get(&tri[1]), index.get(&tri[2])) else {
                continue;
            };
            triangles.extend_from_slice(&[a, b, c]);
            tri_body.push(f.body as u32);
            tri_face.push(f.tag as u32);
        }
    }

    let p = |i: u32| {
        let i = 3 * i as usize;
        [nodes[i], nodes[i + 1], nodes[i + 2]]
    };
    let (mut emin, mut emax, mut esum, mut area) = (f64::INFINITY, 0.0f64, 0.0, 0.0);
    for tri in triangles.chunks_exact(3) {
        let (a, b, c) = (p(tri[0]), p(tri[1]), p(tri[2]));
        let d = |u: [f64; 3], v: [f64; 3]| ((u[0] - v[0]).powi(2) + (u[1] - v[1]).powi(2) + (u[2] - v[2]).powi(2)).sqrt();
        for e in [d(a, b), d(b, c), d(c, a)] {
            emin = emin.min(e);
            emax = emax.max(e);
            esum += e;
        }
        let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
        let cr = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        area += 0.5 * (cr[0] * cr[0] + cr[1] * cr[1] + cr[2] * cr[2]).sqrt();
    }
    let nt = tri_body.len();
    let mut per_body = vec![0; model.bodies.len()];
    for &b in &tri_body {
        per_body[b as usize] += 1;
    }
    let stats = MeshStats {
        n_nodes: nodes.len() / 3,
        n_triangles: nt,
        triangles_per_body: per_body,
        edge_min: if nt > 0 { emin } else { 0.0 },
        edge_max: emax,
        edge_mean: if nt > 0 { esum / (3 * nt) as f64 } else { 0.0 },
        area_total: area,
        failed_faces,
        defective_faces,
        curvature_capped_faces,
        repair_rounds,
        seconds: t0.elapsed().as_secs_f64(),
    };
    Ok(SurfaceMesh { nodes, triangles, tri_body, tri_face, params: params.clone(), stats })
}
