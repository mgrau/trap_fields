//! Loading a STEP model through gmsh/OpenCASCADE, recovering electrode names, and
//! producing a display tessellation (per-face triangles, feature edges, CAD vertices).

use crate::step::{parse_step, StepInfo};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tf_gmsh::{DimTag, Gmsh, ELEMENT_LINE, ELEMENT_TRIANGLE};

#[derive(Debug, thiserror::Error)]
pub enum GeomError {
    #[error(transparent)]
    Gmsh(#[from] tf_gmsh::GmshError),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Invalid(String),
    /// The caller's [`Monitor`] asked to stop.
    #[error("stopped")]
    Stopped,
}

pub type Result<T> = std::result::Result<T, GeomError>;

/// What a long geometry operation reports while it runs.
#[derive(Debug, Clone, Copy)]
pub enum Report<'a> {
    /// A new stage started (shown to the user as is).
    Stage(&'a str),
    /// `done` of `total` items of the current stage are finished.
    Step { done: usize, total: usize },
}

/// Hears [`Report`]s; returning `false` stops the operation there (it then fails with
/// [`GeomError::Stopped`]). A single gmsh call (the OpenCASCADE import, one meshing pass)
/// cannot be interrupted, so operations report between such calls.
pub type Monitor<'a> = &'a mut dyn FnMut(Report) -> bool;

/// A monitor that always goes on.
pub fn no_monitor() -> impl FnMut(Report) -> bool {
    |_| true
}

fn report(m: &mut dyn FnMut(Report) -> bool, r: Report) -> Result<()> {
    if m(r) {
        Ok(())
    } else {
        Err(GeomError::Stopped)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Body {
    pub index: usize,
    pub tag: i32,
    /// Unique display name (STEP name, de-duplicated).
    pub name: String,
    pub step_name: String,
    pub n_faces: usize,
    /// m^3
    pub volume: f64,
    /// m^2
    pub area: f64,
    /// In the STEP frame (m).
    #[serde(skip)]
    pub center_of_mass: [f64; 3],
    /// Bounding-box mismatch (m) between the gmsh volume and the matched STEP solid.
    pub name_match_cost: Option<f64>,
    #[serde(skip)]
    pub faces: Vec<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Face {
    pub tag: i32,
    pub body: usize,
    pub kind: String,
    pub area: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Model {
    #[serde(skip)]
    pub path: PathBuf,
    pub file_name: String,
    pub length_unit: String,
    pub products: Vec<String>,
    pub originating_system: String,
    pub bodies: Vec<Body>,
    pub faces: Vec<Face>,
    #[serde(skip)]
    pub face_body: HashMap<i32, usize>,
    pub n_curves: usize,
    pub n_points: usize,
    pub warnings: Vec<String>,
    pub load_seconds: f64,
    /// Mesh-size fields currently defined in gmsh (removed before re-meshing).
    #[serde(skip)]
    pub fields: Vec<i32>,
}

fn bbox_cost(a: &[f64; 6], lo: &[f64; 3], hi: &[f64; 3]) -> f64 {
    (0..3).map(|k| (a[k] - lo[k]).abs() + (a[k + 3] - hi[k]).abs()).sum()
}

/// Assign STEP solid names to gmsh volumes by comparing bounding boxes.
fn match_names(step: &StepInfo, vol_bbox: &[[f64; 6]], warnings: &mut Vec<String>) -> Vec<(String, Option<f64>)> {
    let (nv, ns) = (vol_bbox.len(), step.solids.len());
    let generic = |i: usize| format!("Body {}", i + 1);
    if ns != nv || step.solids.iter().any(|s| s.n_vertices == 0) {
        if ns == nv {
            warnings.push("Some STEP solids have no vertices; names assigned in file order.".into());
            return step.solids.iter().map(|s| (s.name.clone(), None)).collect();
        }
        warnings.push(format!(
            "STEP file lists {ns} named solids but OpenCASCADE produced {nv} volumes; using generic names."
        ));
        return (0..nv).map(|i| (generic(i), None)).collect();
    }
    let mut pairs: Vec<(f64, usize, usize)> = Vec::with_capacity(nv * ns);
    for (v, bb) in vol_bbox.iter().enumerate() {
        for (s, sol) in step.solids.iter().enumerate() {
            pairs.push((bbox_cost(bb, &sol.vmin, &sol.vmax), v, s));
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut vol_to, mut used) = (vec![None; nv], vec![false; ns]);
    for &(c, v, s) in &pairs {
        if vol_to[v].is_none() && !used[s] {
            vol_to[v] = Some((s, c));
            used[s] = true;
        }
    }
    let mut out = Vec::with_capacity(nv);
    let mut reordered = false;
    for v in 0..nv {
        let (s, c) = vol_to[v].expect("square assignment");
        reordered |= s != v;
        // ambiguity: second-best candidate nearly as good as the assigned one
        let second = pairs.iter().filter(|p| p.1 == v && p.2 != s).map(|p| p.0).fold(f64::INFINITY, f64::min);
        if second < 2.0 * c + 1e-9 {
            warnings.push(format!(
                "Name match for volume {} ('{}') is ambiguous; please verify electrode labels.",
                v + 1,
                step.solids[s].name
            ));
        }
        out.push((step.solids[s].name.clone(), Some(c)));
    }
    if reordered {
        warnings.push("Solid names were matched geometrically and differ from file order.".into());
    }
    out
}

const SHARED_WARNING: &str = "Some faces are shared between bodies (bodies touch); each is meshed once, as part of the first body.";

/// Faces of every body (from its volume tag) and the per-body sums; also whether some
/// face belongs to two bodies.
fn collect_faces(g: &Gmsh, bodies: &mut [Body]) -> Result<(Vec<Face>, HashMap<i32, usize>, bool)> {
    let mut faces = Vec::new();
    let mut face_body = HashMap::new();
    let mut shared = false;
    for (i, b) in bodies.iter_mut().enumerate() {
        let mut ftags: Vec<i32> = g.boundary(&[(3, b.tag)], false, false, false)?.into_iter().map(|(_, t)| t.abs()).collect();
        ftags.sort_unstable();
        ftags.dedup();
        let mut area = 0.0;
        for &f in &ftags {
            let a = g.occ_mass(2, f)?;
            area += a;
            // a face shared by touching bodies is one surface: keep it once (with the first
            // body), or it would be meshed twice into duplicate (singular) BEM panels
            if face_body.insert(f, i).is_none() {
                faces.push(Face { tag: f, body: i, kind: g.entity_type(2, f)?, area: a });
            } else {
                shared = true;
            }
        }
        b.n_faces = ftags.len();
        b.area = area;
        b.volume = g.occ_mass(3, b.tag)?;
        b.center_of_mass = g.occ_center_of_mass(3, b.tag)?;
        b.faces = ftags;
    }
    Ok((faces, face_body, shared))
}

impl Model {
    /// Re-read faces, curves and points from gmsh after the volumes of some bodies were
    /// replaced (their `tag`s updated); body order, names and indices stay.
    pub(crate) fn refresh_topology(&mut self, g: &Gmsh) -> Result<()> {
        let (faces, face_body, shared) = collect_faces(g, &mut self.bodies)?;
        self.faces = faces;
        self.face_body = face_body;
        self.n_curves = g.entities(1)?.len();
        self.n_points = g.entities(0)?.len();
        if shared && !self.warnings.iter().any(|w| w == SHARED_WARNING) {
            self.warnings.push(SHARED_WARNING.into());
        }
        Ok(())
    }
}

pub fn load_step(g: &mut Gmsh, path: &Path) -> Result<Model> {
    load_step_monitored(g, path, &mut no_monitor())
}

/// [`load_step`], reporting its stages to `mon` and stopping between them when it says so.
pub fn load_step_monitored(g: &mut Gmsh, path: &Path, mon: Monitor) -> Result<Model> {
    let t0 = Instant::now();
    report(mon, Report::Stage("reading STEP"))?;
    let bytes = std::fs::read(path)?;
    let text = String::from_utf8_lossy(&bytes);
    if !text.trim_start().starts_with("ISO-10303-21") {
        return Err(GeomError::Invalid(format!("{} is not a STEP (ISO 10303-21) file", path.display())));
    }
    let step = parse_step(&text);
    // the text of a big file is large: don't hold it through the import
    drop(text);
    drop(bytes);
    let mut warnings = Vec::new();
    if step.unit_name.contains("assumed") {
        warnings.push("No length unit found in the STEP file; assuming mm.".into());
    }

    // a single OpenCASCADE call that cannot be interrupted (see `Monitor`)
    report(mon, Report::Stage("importing STEP (OpenCASCADE)"))?;
    g.clear()?;
    g.set_string("Geometry.OCCTargetUnit", "M")?;
    g.set_number("Geometry.OCCImportLabels", 0.0)?;
    g.occ_import_shapes(path)?;
    report(mon, Report::Stage("building the model"))?;
    g.occ_synchronize()?;
    report(mon, Report::Stage("matching body names"))?;

    let vols = g.entities(3)?;
    if vols.is_empty() {
        return Err(GeomError::Invalid(
            "No solid bodies found. Export electrodes as solid bodies (not surfaces/meshes).".into(),
        ));
    }
    let bboxes: Vec<[f64; 6]> = vols.iter().map(|&(d, t)| g.bounding_box(d, t)).collect::<std::result::Result<_, _>>()?;
    let names = match_names(&step, &bboxes, &mut warnings);

    // unique display names
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut bodies = Vec::with_capacity(vols.len());
    for (i, (&(_, tag), (step_name, cost))) in vols.iter().zip(names).enumerate() {
        report(mon, Report::Step { done: i, total: vols.len() })?;
        let base = if step_name.trim().is_empty() { format!("Body {}", i + 1) } else { step_name.trim().to_string() };
        let k = seen.entry(base.to_lowercase()).or_insert(0);
        *k += 1;
        let name = if *k == 1 { base.clone() } else { format!("{base} #{k}") };
        bodies.push(Body {
            index: i,
            tag,
            name,
            step_name: step_name.clone(),
            n_faces: 0,
            volume: 0.0,
            area: 0.0,
            center_of_mass: [0.0; 3],
            name_match_cost: cost,
            faces: Vec::new(),
        });
    }
    report(mon, Report::Stage("measuring bodies"))?;
    let (faces, face_body, shared) = collect_faces(g, &mut bodies)?;
    if shared {
        warnings.push(SHARED_WARNING.into());
    }
    Ok(Model {
        path: path.to_path_buf(),
        file_name: path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
        length_unit: step.unit_name,
        products: step.products,
        originating_system: step.originating_system,
        bodies,
        faces,
        face_body,
        n_curves: g.entities(1)?.len(),
        n_points: g.entities(0)?.len(),
        warnings,
        load_seconds: t0.elapsed().as_secs_f64(),
        fields: Vec::new(),
    })
}

fn triangle_count(g: &Gmsh, face: i32) -> Result<usize> {
    Ok(g.mesh_elements_by_type(ELEMENT_TRIANGLE, face)?.0.len())
}

/// Generate the 2D mesh. If gmsh fails on some face (it aborts the whole run on the
/// first failure), mesh the faces still without triangles in groups, halving a group
/// that fails, and report the faces that fail alone. The fallback reports to `mon` and
/// can be stopped between passes.
pub(crate) fn mesh_surfaces(g: &mut Gmsh, model: &Model, mon: Monitor) -> Result<Vec<i32>> {
    g.set_number("Mesh.MeshOnlyVisible", 0.0)?;
    if g.mesh_generate(2).is_ok() {
        return Ok(Vec::new());
    }
    let all: Vec<DimTag> = model.faces.iter().map(|f| (2, f.tag)).collect();
    g.set_number("Mesh.MeshOnlyVisible", 1.0)?;
    g.set_visibility(&all, false, false)?;
    // gmsh (4.15) crashed re-meshing a group of faces with several threads after a
    // failed pass; the old one-face-at-a-time fallback was effectively serial anyway
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    g.set_number("General.NumThreads", 1.0)?;
    let r = mesh_remaining(g, model, mon);
    g.set_number("General.NumThreads", threads as f64)?;
    // leave gmsh as a full pass would (everything visible), stopped or not
    g.set_visibility(&all, true, false)?;
    g.set_number("Mesh.MeshOnlyVisible", 0.0)?;
    r
}

/// A meshing pass costs about a second on a model of ~1000 faces however few of them
/// are visible: one pass per face (the old fallback) took a quarter of an hour on such a
/// model. Halving failed groups isolates each failing face in O(log n) passes.
fn mesh_remaining(g: &mut Gmsh, model: &Model, mon: Monitor) -> Result<Vec<i32>> {
    let total = model.faces.len();
    let mut unmeshed = Vec::new();
    for f in &model.faces {
        if triangle_count(g, f.tag)? == 0 {
            unmeshed.push(f.tag);
        }
    }
    report(mon, Report::Stage("meshing the faces gmsh skipped"))?;
    let mut failed = Vec::new();
    let mut stack = vec![unmeshed];
    while let Some(group) = stack.pop() {
        let pending: usize = group.len() + stack.iter().map(Vec::len).sum::<usize>();
        report(mon, Report::Step { done: total - pending, total })?;
        let dts: Vec<DimTag> = group.iter().map(|&f| (2, f)).collect();
        g.set_visibility(&dts, true, false)?;
        let ok = g.mesh_generate(2).is_ok();
        g.set_visibility(&dts, false, false)?;
        let mut left = Vec::new();
        for &f in &group {
            if triangle_count(g, f)? == 0 {
                left.push(f);
            }
        }
        if ok || left.len() == 1 && group.len() == 1 {
            failed.extend(left); // a pass that "succeeds" without triangles fails the face too
        } else if left.len() == 1 {
            stack.push(left);
        } else if !left.is_empty() {
            let second = left.split_off(left.len() / 2);
            stack.push(second);
            stack.push(left);
        }
    }
    failed.sort_unstable();
    Ok(failed)
}

pub(crate) fn reset_mesh(g: &mut Gmsh, model: &mut Model) -> Result<()> {
    g.mesh_clear()?;
    for f in model.fields.drain(..) {
        g.field_remove(f)?;
    }
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    g.set_number("General.NumThreads", threads as f64)?;
    g.set_number("Mesh.Algorithm", 6.0)?; // Frontal-Delaunay
    g.set_number("Mesh.MinimumCurvePoints", 3.0)?;
    g.set_number("Mesh.MeshSizeFromCurvature", 0.0)?; // can hang on some B-splines
    Ok(())
}

/// Display tessellation in the STEP frame (m). Vertices are duplicated per face so
/// shading stays sharp across CAD edges.
#[derive(Debug, Default, Clone)]
pub struct Tessellation {
    pub positions: Vec<f64>,
    pub triangles: Vec<u32>,
    pub tri_face: Vec<u32>,
    pub tri_body: Vec<u32>,
    /// Feature-edge segments: pairs of xyz points.
    pub edges: Vec<f64>,
    pub edge_body: Vec<u32>,
    /// CAD vertices (for snapping).
    pub points: Vec<f64>,
    pub point_body: Vec<u32>,
    pub failed_faces: Vec<i32>,
    pub seconds: f64,
}

pub fn tessellate(g: &mut Gmsh, model: &mut Model, max_size: f64) -> Result<Tessellation> {
    tessellate_monitored(g, model, max_size, &mut no_monitor())
}

/// [`tessellate`], reporting its stages and per-face progress to `mon`, and stopping
/// between faces when it says so.
pub fn tessellate_monitored(g: &mut Gmsh, model: &mut Model, max_size: f64, mon: Monitor) -> Result<Tessellation> {
    let t0 = Instant::now();
    report(mon, Report::Stage("tessellating"))?;
    reset_mesh(g, model)?;
    g.set_number("Mesh.MeshSizeMin", 0.0)?;
    g.set_number("Mesh.MeshSizeMax", max_size)?;
    g.set_number("Mesh.MeshSizeFromPoints", 0.0)?;
    g.set_number("Mesh.MeshSizeExtendFromBoundary", 1.0)?;
    g.set_number("Mesh.MinimumCirclePoints", 24.0)?;
    // small holes/bores are often B-spline edges, not circles: give every curve enough points
    g.set_number("Mesh.MinimumCurvePoints", 12.0)?;
    let failed_faces = mesh_surfaces(g, model, mon)?;

    let mut t = Tessellation { failed_faces, ..Default::default() };
    report(mon, Report::Stage("collecting display triangles"))?;
    for (i, f) in model.faces.iter().enumerate() {
        report(mon, Report::Step { done: i, total: model.faces.len() })?;
        let (_, conn) = g.mesh_elements_by_type(ELEMENT_TRIANGLE, f.tag)?;
        if conn.is_empty() {
            continue;
        }
        let (tags, xyz) = g.mesh_nodes(2, f.tag, true)?;
        let base = (t.positions.len() / 3) as u32;
        let local: HashMap<usize, u32> = tags.iter().enumerate().map(|(i, &n)| (n, base + i as u32)).collect();
        t.positions.extend_from_slice(&xyz);
        for tri in conn.chunks_exact(3) {
            let (Some(&a), Some(&b), Some(&c)) = (local.get(&tri[0]), local.get(&tri[1]), local.get(&tri[2])) else {
                continue;
            };
            t.triangles.extend_from_slice(&[a, b, c]);
            t.tri_face.push(f.tag as u32);
            t.tri_body.push(f.body as u32);
        }
    }
    report(mon, Report::Stage("collecting edges"))?;
    for b in &model.bodies {
        report(mon, Report::Step { done: b.index, total: model.bodies.len() })?;
        let dts: Vec<DimTag> = b.faces.iter().map(|&f| (2, f)).collect();
        let mut curves: Vec<i32> = g.boundary(&dts, false, false, false)?.into_iter().map(|(_, c)| c.abs()).collect();
        curves.sort_unstable();
        curves.dedup();
        for c in curves {
            let (_, conn) = g.mesh_elements_by_type(ELEMENT_LINE, c)?;
            if conn.is_empty() {
                continue;
            }
            let (tags, xyz) = g.mesh_nodes(1, c, true)?;
            let idx: HashMap<usize, usize> = tags.iter().enumerate().map(|(i, &n)| (n, i)).collect();
            for seg in conn.chunks_exact(2) {
                let (Some(&i), Some(&j)) = (idx.get(&seg[0]), idx.get(&seg[1])) else { continue };
                t.edges.extend_from_slice(&xyz[3 * i..3 * i + 3]);
                t.edges.extend_from_slice(&xyz[3 * j..3 * j + 3]);
                t.edge_body.push(b.index as u32);
            }
        }
        for (d, p) in g.boundary(&[(3, b.tag)], false, false, true)? {
            if d == 0 {
                let xyz = g.value(0, p.abs(), &[])?;
                if xyz.len() == 3 {
                    t.points.extend_from_slice(&xyz);
                    t.point_body.push(b.index as u32);
                }
            }
        }
    }
    t.seconds = t0.elapsed().as_secs_f64();
    Ok(t)
}
