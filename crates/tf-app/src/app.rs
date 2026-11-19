//! The application: project state plus every operation, run synchronously on the thread
//! that owns gmsh. Front ends (the native HTTP server, the WebAssembly engine) call
//! [`App::command`] and relay [`Event`]s from long operations to the GUI.

use crate::bundle;
use crate::project::{step_files, Label, Project, Role};
use crate::solve::{self, SolveParams, SolveResult, TrapParams};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tf_geom::{load_step, mesh_bem, metrics, tessellate, MeshParams, MeshStats, Model, Rigid, SurfaceMesh, Tessellation, TransformOp};
use tf_gmsh::Gmsh;

/// Progress from a running operation.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// A long operation started.
    Start { kind: String },
    /// The running operation entered a new stage.
    Stage { stage: String },
    /// GMRES iteration: estimated relative residual per right-hand side.
    Gmres { iteration: usize, residuals: Vec<f64> },
    /// One finished level of a convergence study.
    Study { level: Value },
}

/// Sink for [`Event`]s.
pub type Sink<'a> = &'a (dyn Fn(Event) + Sync);

/// Outcome of the last finished operation.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum JobStatus {
    #[default]
    Idle,
    Done { kind: String, seconds: f64 },
    Failed { kind: String, error: String },
    /// Stopped on request (see [`App::stop_handle`]).
    Stopped { kind: String },
}

#[derive(Debug)]
pub struct AppError {
    /// HTTP-style status (400 bad request, 404 missing, 409 conflict).
    pub status: u16,
    pub message: String,
}

impl AppError {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        AppError { status, message: message.into() }
    }
}

impl<E: std::fmt::Display> From<E> for AppError {
    fn from(e: E) -> Self {
        AppError { status: 400, message: format!("{e:#}") }
    }
}

pub type AppResult<T> = Result<T, AppError>;

/// Result of a command: JSON, or a binary buffer (geometry, charge densities).
pub enum Reply {
    Json(Value),
    Binary(Vec<u8>),
}

/// Derived geometry in the current (transformed) frame.
#[derive(Clone)]
struct GeomCache {
    body_bbox: Vec<[f64; 6]>,
    /// Closest distance (m) and point from the frame origin to each body's surface.
    closest: Vec<Option<(f64, [f64; 3])>>,
    bbox: [f64; 6],
}

fn compute_cache(model: &Model, tess: &Tessellation, frame: &Rigid) -> GeomCache {
    let n = model.bodies.len();
    let mut pos = tess.positions.clone();
    frame.apply_flat(&mut pos);
    let empty = [f64::INFINITY, f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut body_bbox = vec![empty; n];
    for (t, tri) in tess.triangles.chunks_exact(3).enumerate() {
        let b = &mut body_bbox[tess.tri_body[t] as usize];
        for &v in tri {
            for k in 0..3 {
                let x = pos[3 * v as usize + k];
                b[k] = b[k].min(x);
                b[k + 3] = b[k + 3].max(x);
            }
        }
    }
    let mut bbox = empty;
    for b in &body_bbox {
        for k in 0..3 {
            bbox[k] = bbox[k].min(b[k]);
            bbox[k + 3] = bbox[k + 3].max(b[k + 3]);
        }
    }
    let closest = metrics::closest_per_group([0.0; 3], &pos, &tess.triangles, &tess.tri_body, n);
    GeomCache { body_bbox, closest, bbox }
}

pub struct App {
    pub dir: PathBuf,
    gmsh: Gmsh,
    pub project: Project,
    model: Option<Model>,
    tess: Option<Tessellation>,
    tess_version: u64,
    mesh: Option<SurfaceMesh>,
    mesh_version: u64,
    solution: Option<SolveResult>,
    solution_version: u64,
    cache: Option<GeomCache>,
    job: JobStatus,
    /// Result of the last convergence study.
    study: Option<Value>,
    /// Problems worth showing that did not fail an operation (e.g. outputs not restored).
    notes: Vec<String>,
    /// Set from another thread to stop the running operation at its next checkpoint.
    stop: Arc<AtomicBool>,
}

fn push_u32(buf: &mut Vec<u8>, v: &[u32]) {
    for x in v {
        buf.extend_from_slice(&x.to_le_bytes());
    }
}

fn push_f32(buf: &mut Vec<u8>, v: impl IntoIterator<Item = f64>) {
    for x in v {
        buf.extend_from_slice(&(x as f32).to_le_bytes());
    }
}

/// TFM1 | u32 n_nodes | u32 n_tri | f32 nodes[3n] (m) | u32 tri[3t] | u32 tri_body[t] | u32 tri_face[t]
pub fn encode_mesh(m: &SurfaceMesh) -> Vec<u8> {
    let (nn, nt) = (m.nodes.len() / 3, m.tri_body.len());
    let mut buf = Vec::with_capacity(12 + 4 * (3 * nn + 5 * nt));
    buf.extend_from_slice(b"TFM1");
    push_u32(&mut buf, &[nn as u32, nt as u32]);
    push_f32(&mut buf, m.nodes.iter().copied());
    push_u32(&mut buf, &m.triangles);
    push_u32(&mut buf, &m.tri_body);
    push_u32(&mut buf, &m.tri_face);
    buf
}

#[derive(Serialize)]
struct MeshFileHeader<'a> {
    format: &'static str,
    step_file: Option<&'a str>,
    transforms: &'a [TransformOp],
    bodies: Vec<&'a str>,
    labels: Vec<Option<&'a Label>>,
    params: &'a MeshParams,
    stats: &'a MeshStats,
}

#[derive(Deserialize)]
struct SavedMeshHeader {
    step_file: Option<String>,
    transforms: Vec<TransformOp>,
    bodies: Vec<String>,
    params: MeshParams,
    stats: MeshStats,
}

#[derive(Deserialize)]
struct OpenArgs {
    file: String,
}

#[derive(Deserialize)]
struct UploadArgs {
    name: String,
}

#[derive(Deserialize)]
struct EvalArgs {
    /// Trap-frame points (m).
    points: Vec<[f64; 3]>,
    /// Electrode voltages (V); electrodes not listed are at 0 V.
    voltages: BTreeMap<String, f64>,
}

#[derive(Deserialize)]
struct SweepArgs {
    params: TrapParams,
    variable: solve::SweepVar,
    values: Vec<f64>,
}

#[derive(Deserialize)]
struct ChainArgs {
    params: TrapParams,
    n: usize,
}

#[derive(Deserialize)]
struct StudyArgs {
    /// Mesh sizing at scale 1; each level multiplies h_min and h_max by its scale.
    base: MeshParams,
    scales: Vec<f64>,
    solve: SolveParams,
    /// Drive for the single-ion quantities (secular frequencies), if any.
    #[serde(default)]
    trap: Option<TrapParams>,
}

#[derive(Deserialize)]
struct SliceArgs {
    params: TrapParams,
    /// 0, 1, 2: plane normal along x, y, z.
    axis: usize,
    center: [f64; 3],
    half: f64,
    n: usize,
}

const MAX_EVAL_POINTS: usize = 200_000;

impl App {
    /// Start gmsh and load the project file in `dir` (created on first save).
    pub fn new(dir: &Path) -> anyhow::Result<App> {
        let gmsh = Gmsh::initialize()?;
        std::fs::create_dir_all(dir)?;
        Ok(App {
            dir: dir.to_path_buf(),
            gmsh,
            project: Project::load(dir)?,
            model: None,
            tess: None,
            tess_version: 0,
            mesh: None,
            mesh_version: 0,
            solution: None,
            solution_version: 0,
            cache: None,
            job: JobStatus::Idle,
            study: None,
            notes: Vec::new(),
            stop: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Flag that stops the running long operation when set (from any thread). The
    /// operation polls it between stages and in its inner loops; it is cleared when the
    /// next operation starts.
    pub fn stop_handle(&self) -> Arc<AtomicBool> {
        self.stop.clone()
    }

    fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    fn check_stop(&self) -> AppResult<()> {
        if self.stop_requested() {
            Err(AppError::new(499, "stopped"))
        } else {
            Ok(())
        }
    }

    /// Kinds of commands that run long enough to report progress.
    pub fn is_long(cmd: &str) -> bool {
        matches!(cmd, "startup" | "open" | "upload" | "mesh" | "solve" | "study" | "import_project")
    }

    /// Run a command. `data` carries binary input (uploads).
    pub fn command(&mut self, cmd: &str, args: Value, data: Option<&[u8]>, sink: Sink) -> AppResult<Reply> {
        let json = |v: Value| Ok(Reply::Json(v));
        match cmd {
            "state" => json(self.state()),
            "startup" => {
                self.startup(sink);
                json(self.state())
            }
            "open" => {
                let a: OpenArgs = serde_json::from_value(args)?;
                self.tracked("load", sink, |app, sink| app.open(&a.file, sink))?;
                json(self.state())
            }
            "upload" => {
                let a: UploadArgs = serde_json::from_value(args)?;
                let data = data.ok_or_else(|| AppError::new(400, "no file data"))?;
                let name = self.store_upload(&a.name, data)?;
                self.tracked("load", sink, |app, sink| app.open(&name, sink))?;
                let mut v = self.state();
                v["uploaded_as"] = json!(name);
                json(v)
            }
            "tess" => Ok(Reply::Binary(self.tess_binary()?)),
            "transform" => {
                let op: TransformOp = serde_json::from_value(args)?;
                self.edit_transforms(|t| t.push(op))?;
                json(self.state())
            }
            "transform_undo" => {
                self.edit_transforms(|t| {
                    t.pop();
                })?;
                json(self.state())
            }
            "transform_reset" => {
                self.edit_transforms(|t| t.clear())?;
                json(self.state())
            }
            "labels" => {
                let labels: BTreeMap<String, Label> = serde_json::from_value(args)?;
                self.set_labels(labels)?;
                json(self.state())
            }
            "mesh" => {
                let params: MeshParams = serde_json::from_value(args)?;
                self.tracked("mesh", sink, |app, sink| app.mesh(params, sink))?;
                json(self.state())
            }
            "mesh_data" => {
                let m = self.mesh.as_ref().ok_or_else(|| AppError::new(404, "no mesh"))?;
                Ok(Reply::Binary(encode_mesh(m)))
            }
            "solve" => {
                let params: SolveParams = serde_json::from_value(args)?;
                self.tracked("solve", sink, |app, sink| app.solve(params, sink))?;
                json(self.state())
            }
            "solution" => {
                let r = self.solution()?;
                let mut v = r.summary(self.solution_version);
                v["stale"] = json!(self.solution_stale());
                json(v)
            }
            "sigma" => Ok(Reply::Binary(solve::encode_sigma(self.solution()?))),
            "eval" => {
                let a: EvalArgs = serde_json::from_value(args)?;
                json(self.eval(a)?)
            }
            "trap" => {
                let p: TrapParams = serde_json::from_value(args)?;
                self.project.trap_params = Some(TrapParams { depth: false, ..p.clone() });
                self.project.save(&self.dir)?;
                json(self.solution()?.analyze(&p)?)
            }
            "sweep" => {
                let a: SweepArgs = serde_json::from_value(args.clone())?;
                let r = self.solution()?.sweep(&a.params, &a.variable, &a.values)?;
                self.save_result("sweep", args, &r);
                json(r)
            }
            "chain" => {
                let a: ChainArgs = serde_json::from_value(args.clone())?;
                let r = self.solution()?.chain(&a.params, a.n)?;
                self.save_result("chain", args, &r);
                json(r)
            }
            "results" => json(self.saved_results()),
            "export_project" => Ok(Reply::Binary(bundle::write_tar(&self.dir, &bundle::project_files(&self.dir, self.project.step_file.as_deref()))?)),
            "import_project" => {
                let data = data.ok_or_else(|| AppError::new(400, "no bundle data"))?;
                self.tracked("load", sink, |app, sink| app.import_project(data, sink))?;
                json(self.state())
            }
            "study" => {
                let a: StudyArgs = serde_json::from_value(args)?;
                self.tracked("study", sink, |app, sink| app.study(a, sink))?;
                json(self.state())
            }
            "slice" => {
                let a: SliceArgs = serde_json::from_value(args.clone())?;
                let r = self.solution()?.slice(&a.params, a.axis, a.center, a.half, a.n)?;
                self.save_result("slice", args, &r);
                json(r)
            }
            _ => Err(AppError::new(404, format!("unknown command '{cmd}'"))),
        }
    }

    /// Record the outcome of a long operation in the job status.
    fn tracked(&mut self, kind: &str, sink: Sink, f: impl FnOnce(&mut App, Sink) -> AppResult<f64>) -> AppResult<()> {
        self.stop.store(false, Ordering::Relaxed);
        sink(Event::Start { kind: kind.into() });
        let t = Instant::now();
        match f(self, sink) {
            Ok(seconds) => {
                let seconds = if seconds > 0.0 { seconds } else { t.elapsed().as_secs_f64() };
                self.job = JobStatus::Done { kind: kind.into(), seconds };
                Ok(())
            }
            Err(_) if self.stop_requested() => {
                self.job = JobStatus::Stopped { kind: kind.into() };
                Err(AppError::new(499, "stopped"))
            }
            Err(e) => {
                self.job = JobStatus::Failed { kind: kind.into(), error: e.message.clone() };
                Err(e)
            }
        }
    }

    fn frame(&self) -> Rigid {
        Rigid::from_ops(&self.project.transforms)
    }

    fn refresh_cache(&mut self) {
        self.cache = match (&self.model, &self.tess) {
            (Some(m), Some(t)) => Some(compute_cache(m, t, &self.frame())),
            _ => None,
        };
    }

    /// The mesh (and with it any solution) no longer matches the geometry or frame.
    fn drop_mesh(&mut self) {
        self.mesh = None;
        self.solution = None;
    }

    /// Whether the electrode grouping changed since the solution was computed.
    fn solution_stale(&self) -> bool {
        match (&self.solution, &self.model) {
            (Some(r), Some(m)) => solve::electrodes(m, &self.project.labels).2 != r.signature,
            _ => false,
        }
    }

    fn solution(&self) -> AppResult<&SolveResult> {
        self.solution.as_ref().ok_or_else(|| AppError::new(404, "no solution: solve first"))
    }

    /// Suggested mesh parameters from the geometry around the frame origin.
    fn mesh_defaults(&self) -> Option<MeshParams> {
        let c = self.cache.as_ref()?;
        let near = c.closest.iter().flatten().map(|x| x.0).fold(f64::INFINITY, f64::min);
        let diag = ((0..3).map(|k| (c.bbox[k + 3] - c.bbox[k]).powi(2)).sum::<f64>()).sqrt();
        if !near.is_finite() || near <= 0.0 {
            return None;
        }
        // round to 2 significant figures so the suggested values read nicely
        let r2 = |x: f64| {
            let p = 10f64.powi(x.log10().floor() as i32 - 1);
            (x / p).round() * p
        };
        let h_min = r2(near / 25.0);
        // the size grows by (h_max − h_min)/(dist_max − dist_min) per unit distance: about 0.15
        // with dist_max = 25 near, i.e. ~8° per panel seen from the center. A steeper
        // grading (dist_max = 10 near, ~24°) under-resolves electrodes a few mm away, such as
        // endcaps: in the convergence study of the four-blade trap it put the axial frequency
        // 6 % high, against 2 % here for 1.5× the panels.
        Some(MeshParams { center: [0.0; 3], h_min, h_max: r2(diag / 25.0).max(h_min), dist_min: r2(near), dist_max: r2(25.0 * near), min_circle_points: 16 })
    }

    pub fn state(&self) -> Value {
        let c = self.cache.as_ref();
        let bodies: Vec<Value> = self
            .model
            .iter()
            .flat_map(|m| m.bodies.iter())
            .map(|b| {
                json!({
                    "index": b.index,
                    "name": b.name,
                    "step_name": b.step_name,
                    "n_faces": b.n_faces,
                    "volume": b.volume,
                    "area": b.area,
                    "name_match_cost": b.name_match_cost,
                    "bbox": c.map(|c| c.body_bbox[b.index]),
                    "closest": c.and_then(|c| c.closest[b.index]).map(|(d, p)| json!({"distance": d, "point": p})),
                    "label": self.project.labels.get(&b.name),
                })
            })
            .collect();
        let frame = self.frame();
        json!({
            "project_dir": self.dir.display().to_string(),
            "files": step_files(&self.dir),
            "project": self.project,
            "model": self.model.as_ref().map(|m| json!({
                "file_name": m.file_name,
                "length_unit": m.length_unit,
                "products": m.products,
                "originating_system": m.originating_system,
                "n_faces": m.faces.len(),
                "n_curves": m.n_curves,
                "n_points": m.n_points,
                "warnings": m.warnings,
                "load_seconds": m.load_seconds,
            })),
            "bodies": bodies,
            "bbox": c.map(|c| c.bbox),
            "frame": {"r": frame.r, "t": frame.t},
            "tess": self.tess.as_ref().map(|t| json!({
                "version": self.tess_version,
                "n_triangles": t.tri_body.len(),
                "failed_faces": t.failed_faces,
                "seconds": t.seconds,
            })),
            "mesh": self.mesh.as_ref().map(|m| json!({
                "version": self.mesh_version,
                "params": m.params,
                "stats": m.stats,
            })),
            "mesh_defaults": self.mesh_defaults(),
            "solve": self.solution.as_ref().map(|r| json!({
                "version": self.solution_version,
                "mesh_version": r.mesh_version,
                "n_panels": r.problem.panels.len(),
                "seconds": r.seconds,
                "stale": self.solution_stale(),
            })),
            "job": self.job,
            "study": self.study,
            "notes": self.notes,
        })
    }

    /// Re-open the project's STEP file and restore the saved mesh and solution.
    pub fn startup(&mut self, sink: Sink) {
        let Some(file) = self.project.step_file.clone() else { return };
        if !self.dir.join(&file).exists() {
            return;
        }
        if self.tracked("load", sink, |app, sink| app.open(&file, sink)).is_err() {
            return;
        }
        sink(Event::Stage { stage: "restoring mesh and solution".into() });
        if let Err(e) = self.restore_outputs() {
            self.notes.push(format!("saved mesh/solution not restored: {}", e.message));
        }
        self.restore_study();
    }

    // ------------------------------------------------------------ saved results

    /// Results computed from the current solution (sweeps, chains, slices) are saved in
    /// results/<name>.json with the solution's id, so they come back with it.
    fn save_result(&mut self, name: &str, args: Value, result: &Value) {
        let Some(sol) = &self.solution else { return };
        let v = json!({ "solution_id": sol.id, "args": args, "result": result });
        let out = self.dir.join("results");
        let r = std::fs::create_dir_all(&out).and_then(|_| std::fs::write(out.join(format!("{name}.json")), serde_json::to_vec(&v).unwrap_or_default()));
        if let Err(e) = r {
            self.notes.push(format!("could not save the {name} result: {e}"));
        }
    }

    /// {sweep, chain, slice}: each {args, result} if saved for the current solution, else null.
    fn saved_results(&self) -> Value {
        let id = self.solution.as_ref().map(|s| s.id);
        let mut out = json!({});
        for name in ["sweep", "chain", "slice"] {
            let v: Option<Value> = std::fs::read(self.dir.join("results").join(format!("{name}.json"))).ok().and_then(|b| serde_json::from_slice(&b).ok());
            out[name] = match v {
                Some(v) if id.is_some() && v["solution_id"].as_u64() == id => json!({ "args": v["args"], "result": v["result"] }),
                _ => Value::Null,
            };
        }
        out
    }

    /// What a convergence study depends on besides the mesh sizes it was given.
    fn study_key(&self) -> Value {
        json!({ "step_file": self.project.step_file, "transforms": self.project.transforms, "labels": self.project.labels })
    }

    fn save_study(&mut self) {
        let Some(study) = &self.study else { return };
        let v = json!({ "key": self.study_key(), "study": study });
        let out = self.dir.join("results");
        if let Err(e) = std::fs::create_dir_all(&out).and_then(|_| std::fs::write(out.join("study.json"), serde_json::to_vec(&v).unwrap_or_default())) {
            self.notes.push(format!("could not save the study: {e}"));
        }
    }

    fn restore_study(&mut self) {
        let v: Option<Value> = std::fs::read(self.dir.join("results/study.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
        self.study = v.filter(|v| v["key"] == self.study_key()).map(|v| v["study"].clone());
    }

    /// Replace the project with a bundle (see [`bundle`]) and load it.
    fn import_project(&mut self, data: &[u8], sink: Sink) -> AppResult<f64> {
        let t = Instant::now();
        sink(Event::Stage { stage: "unpacking".into() });
        let entries = bundle::read_tar(data)?;
        let mut names = std::collections::HashSet::new();
        if let Some((n, _)) = entries.iter().find(|(n, _)| !names.insert(n.as_str())) {
            return Err(AppError::new(400, format!("the bundle contains {n} twice")));
        }
        let project: Project = entries
            .iter()
            .find(|(n, _)| n == crate::project::PROJECT_FILE)
            .map(|(_, b)| serde_json::from_slice(b))
            .transpose()
            .map_err(|e| AppError::new(400, format!("bad project file in the bundle: {e}")))?
            .unwrap_or_default();
        if let Some(f) = &project.step_file {
            if !bundle::allowed(f) || f.contains('/') || f.contains('\\') || f == crate::project::PROJECT_FILE {
                return Err(AppError::new(400, format!("the bundle's project names an invalid STEP file: {f}")));
            }
            if !entries.iter().any(|(n, _)| n == f) && !self.dir.join(f).exists() {
                return Err(AppError::new(400, format!("the bundle does not contain its STEP file {f}")));
            }
        }
        // outputs of the old project must not mix with the new one
        for d in bundle::OUTPUT_DIRS {
            let _ = std::fs::remove_dir_all(self.dir.join(d));
        }
        for (name, bytes) in &entries {
            let path = self.dir.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, bytes)?;
        }
        self.project = Project::load(&self.dir)?;
        self.model = None;
        self.tess = None;
        self.cache = None;
        self.drop_mesh();
        self.study = None;
        self.notes.clear();
        if let Some(file) = self.project.step_file.clone() {
            self.open(&file, sink)?;
            sink(Event::Stage { stage: "restoring mesh and solution".into() });
            if let Err(e) = self.restore_outputs() {
                self.notes.push(format!("saved mesh/solution not restored: {}", e.message));
            }
            self.restore_study();
        }
        Ok(t.elapsed().as_secs_f64())
    }

    fn project_file(&self, name: &str) -> AppResult<PathBuf> {
        let base = Path::new(name).file_name().ok_or_else(|| AppError::new(400, "invalid file name"))?;
        let lower = base.to_string_lossy().to_lowercase();
        if !(lower.ends_with(".step") || lower.ends_with(".stp")) {
            return Err(AppError::new(400, "only .step / .stp files are supported"));
        }
        Ok(self.dir.join(base))
    }

    /// Load a STEP file from the project directory; returns the elapsed seconds.
    fn open(&mut self, file: &str, sink: Sink) -> AppResult<f64> {
        let path = self.project_file(file)?;
        if !path.exists() {
            return Err(AppError::new(404, format!("{file} not found in the project directory")));
        }
        sink(Event::Stage { stage: "reading STEP".into() });
        self.model = None;
        let mut model = load_step(&mut self.gmsh, &path)?;
        sink(Event::Stage { stage: "tessellating".into() });
        let bb = self.gmsh.bounding_box(-1, -1)?;
        let diag = ((0..3).map(|k| (bb[k + 3] - bb[k]).powi(2)).sum::<f64>()).sqrt();
        let tess = tessellate(&mut self.gmsh, &mut model, diag / 100.0)?;
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        if self.project.step_file.as_deref() != Some(file.as_str()) {
            self.project.transforms.clear();
            self.project.mesh_params = None;
        }
        self.project.step_file = Some(file);
        for b in &model.bodies {
            self.project.labels.entry(b.name.clone()).or_insert_with(|| Label { electrode: b.name.clone(), role: Role::Unassigned });
        }
        let seconds = model.load_seconds + tess.seconds;
        self.model = Some(model);
        self.tess = Some(tess);
        self.tess_version += 1;
        self.drop_mesh();
        self.refresh_cache();
        self.notes.clear();
        self.project.save(&self.dir)?;
        self.restore_study();
        Ok(seconds)
    }

    /// Save an uploaded STEP file into the project directory, never overwriting a
    /// different file of the same name; returns the stored name.
    fn store_upload(&mut self, name: &str, data: &[u8]) -> AppResult<String> {
        let mut path = self.project_file(name)?;
        if !data.starts_with(b"ISO-10303-21") {
            return Err(AppError::new(400, "not a STEP file (missing ISO-10303-21 header)"));
        }
        if path.exists() && std::fs::read(&path)? != data {
            let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
            let ext = path.extension().unwrap().to_string_lossy().into_owned();
            let mut n = 2;
            while path.exists() {
                path = self.dir.join(format!("{stem}-{n}.{ext}"));
                n += 1;
            }
        }
        if !path.exists() {
            std::fs::write(&path, data)?;
        }
        Ok(path.file_name().unwrap().to_string_lossy().into_owned())
    }

    /// TFT1 | nv nt ne np | f32 pos[3nv] | u32 tri[3nt] | u32 tri_body[nt] | u32 tri_face[nt]
    ///      | f32 edges[6ne] | u32 edge_body[ne] | f32 points[3np] | u32 point_body[np]
    /// Coordinates in meters, transformed frame.
    fn tess_binary(&self) -> AppResult<Vec<u8>> {
        let tess = self.tess.as_ref().ok_or_else(|| AppError::new(404, "no model loaded"))?;
        let frame = self.frame();
        let mut pos = tess.positions.clone();
        frame.apply_flat(&mut pos);
        let mut edges = tess.edges.clone();
        frame.apply_flat(&mut edges);
        let mut pts = tess.points.clone();
        frame.apply_flat(&mut pts);
        let (nv, nt, ne, np) = (pos.len() / 3, tess.tri_body.len(), tess.edge_body.len(), tess.point_body.len());
        let mut buf = Vec::with_capacity(20 + 4 * (3 * nv + 5 * nt + 7 * ne + 4 * np));
        buf.extend_from_slice(b"TFT1");
        push_u32(&mut buf, &[nv as u32, nt as u32, ne as u32, np as u32]);
        push_f32(&mut buf, pos);
        push_u32(&mut buf, &tess.triangles);
        push_u32(&mut buf, &tess.tri_body);
        push_u32(&mut buf, &tess.tri_face);
        push_f32(&mut buf, edges);
        push_u32(&mut buf, &tess.edge_body);
        push_f32(&mut buf, pts);
        push_u32(&mut buf, &tess.point_body);
        Ok(buf)
    }

    fn edit_transforms(&mut self, f: impl FnOnce(&mut Vec<TransformOp>)) -> AppResult<()> {
        if self.model.is_none() {
            return Err(AppError::new(400, "no model loaded"));
        }
        f(&mut self.project.transforms);
        // a mesh (and its solution) lives in the frame it was generated in
        self.drop_mesh();
        self.refresh_cache();
        self.project.save(&self.dir)?;
        Ok(())
    }

    fn set_labels(&mut self, labels: BTreeMap<String, Label>) -> AppResult<()> {
        for (k, v) in labels {
            let electrode = v.electrode.trim().to_string();
            if electrode.is_empty() {
                return Err(AppError::new(400, format!("electrode name for '{k}' is empty")));
            }
            self.project.labels.insert(k, Label { electrode, role: v.role });
        }
        self.project.save(&self.dir)?;
        Ok(())
    }

    fn mesh(&mut self, params: MeshParams, sink: Sink) -> AppResult<f64> {
        params.validate()?;
        let frame = self.frame();
        let model = self.model.as_mut().ok_or_else(|| AppError::new(400, "no model loaded"))?;
        self.project.mesh_params = Some(params.clone());
        self.project.save(&self.dir)?;
        sink(Event::Stage { stage: "meshing".into() });
        let mesh = mesh_bem(&mut self.gmsh, model, &params, &frame)?;
        // gmsh cannot be interrupted: a stop requested while it ran discards the result
        self.check_stop()?;
        if let Err(e) = self.save_mesh(&mesh) {
            self.notes.push(format!("could not save mesh: {e:#}"));
        }
        let seconds = mesh.stats.seconds;
        self.mesh = Some(mesh);
        self.mesh_version += 1;
        self.solution = None;
        Ok(seconds)
    }

    fn save_mesh(&self, m: &SurfaceMesh) -> anyhow::Result<()> {
        let out = self.dir.join("mesh");
        std::fs::create_dir_all(&out)?;
        let model = self.model.as_ref().ok_or_else(|| anyhow::anyhow!("no model"))?;
        let header = MeshFileHeader {
            format: "TFM1 little-endian: magic, u32 n_nodes, u32 n_tri, f32 nodes[3n] (m), u32 tri[3t], u32 tri_body[t], u32 tri_face[t]",
            step_file: self.project.step_file.as_deref(),
            transforms: &self.project.transforms,
            bodies: model.bodies.iter().map(|b| b.name.as_str()).collect(),
            labels: model.bodies.iter().map(|b| self.project.labels.get(&b.name)).collect(),
            params: &m.params,
            stats: &m.stats,
        };
        std::fs::write(out.join("surface_mesh.json"), serde_json::to_string_pretty(&header)?)?;
        std::fs::write(out.join("surface_mesh.bin"), encode_mesh(m))?;
        // full-precision node coordinates, used to restore the mesh exactly
        let raw: Vec<u8> = m.nodes.iter().flat_map(|x| x.to_le_bytes()).collect();
        std::fs::write(out.join("surface_mesh_nodes.f64"), raw)?;
        Ok(())
    }

    /// The saved mesh, if it belongs to the current STEP file, frame and bodies.
    fn load_mesh(&self) -> anyhow::Result<Option<SurfaceMesh>> {
        let d = self.dir.join("mesh");
        let (Ok(hdr), Ok(bin)) = (std::fs::read_to_string(d.join("surface_mesh.json")), std::fs::read(d.join("surface_mesh.bin"))) else {
            return Ok(None);
        };
        let h: SavedMeshHeader = serde_json::from_str(&hdr)?;
        let model = self.model.as_ref().ok_or_else(|| anyhow::anyhow!("no model"))?;
        let names: Vec<&str> = model.bodies.iter().map(|b| b.name.as_str()).collect();
        if h.step_file != self.project.step_file || h.transforms != self.project.transforms || h.bodies != names {
            return Ok(None);
        }
        anyhow::ensure!(bin.len() >= 12 && &bin[0..4] == b"TFM1", "bad mesh file");
        let u32s = |o: usize, n: usize| -> Vec<u32> { (0..n).map(|i| u32::from_le_bytes(bin[o + 4 * i..o + 4 * i + 4].try_into().unwrap())).collect() };
        let (nn, nt) = (u32s(4, 1)[0] as usize, u32s(8, 1)[0] as usize);
        anyhow::ensure!(bin.len() == 12 + 4 * (3 * nn + 5 * nt), "truncated mesh file");
        let mut o = 12;
        let mut nodes: Vec<f64> = (0..3 * nn).map(|i| f32::from_le_bytes(bin[o + 4 * i..o + 4 * i + 4].try_into().unwrap()) as f64).collect();
        o += 12 * nn;
        let triangles = u32s(o, 3 * nt);
        o += 12 * nt;
        let tri_body = u32s(o, nt);
        o += 4 * nt;
        let tri_face = u32s(o, nt);
        if let Ok(raw) = std::fs::read(d.join("surface_mesh_nodes.f64")) {
            if raw.len() == 24 * nn {
                nodes = raw.chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap())).collect();
            }
        }
        anyhow::ensure!(
            triangles.iter().all(|&i| (i as usize) < nn) && tri_body.iter().all(|&b| (b as usize) < names.len()),
            "inconsistent mesh file"
        );
        Ok(Some(SurfaceMesh { nodes, triangles, tri_body, tri_face, params: h.params, stats: h.stats }))
    }

    /// Restore the saved mesh and solution when they match the current project.
    fn restore_outputs(&mut self) -> AppResult<()> {
        let Some(mesh) = self.load_mesh()? else { return Ok(()) };
        self.mesh_version += 1;
        let model = self.model.as_ref().ok_or_else(|| AppError::new(400, "no model"))?;
        let res = solve::load(&self.dir, &mesh, self.mesh_version, model, &self.project)?;
        self.mesh = Some(mesh);
        if let Some(res) = res {
            self.solution_version += 1;
            self.solution = Some(res);
        }
        Ok(())
    }

    fn solve(&mut self, params: SolveParams, sink: Sink) -> AppResult<f64> {
        let mesh = self.mesh.as_ref().ok_or_else(|| AppError::new(400, "generate a surface mesh first"))?;
        let model = self.model.as_ref().ok_or_else(|| AppError::new(400, "no model loaded"))?;
        self.project.solve_params = Some(params.clone());
        self.project.save(&self.dir)?;
        let flag = self.stop.clone();
        let res = solve::run(mesh, self.mesh_version, model, &self.project.labels, params, sink, &move || flag.load(Ordering::Relaxed))?;
        self.solution_version += 1;
        if let Err(e) = solve::save(&self.dir, &res, self.solution_version, &self.project, mesh) {
            self.notes.push(format!("could not save solution: {e:#}"));
        }
        let seconds = res.seconds;
        self.solution = Some(res);
        Ok(seconds)
    }

    /// Potential and gradient at points by direct integration over the surface charges,
    /// and (inside the convergence sphere) from the expansion.
    fn eval(&self, a: EvalArgs) -> AppResult<Value> {
        if a.points.len() > MAX_EVAL_POINTS {
            return Err(AppError::new(400, format!("at most {MAX_EVAL_POINTS} points per request")));
        }
        let r = self.solution()?;
        let (u, exp) = r.combine(&a.voltages)?;
        let direct = tf_bem::evaluate(&r.problem.panels, &u, &a.points);
        let expansion: Option<Vec<Value>> = exp.map(|e| {
            a.points
                .iter()
                .map(|&p| {
                    let d = ((p[0] - e.center[0]).powi(2) + (p[1] - e.center[1]).powi(2) + (p[2] - e.center[2]).powi(2)).sqrt();
                    if d < r.r_min {
                        let (phi, grad) = e.potential_grad(p);
                        json!({"phi": phi, "grad": grad})
                    } else {
                        Value::Null
                    }
                })
                .collect()
        });
        Ok(json!({
            "phi": direct.iter().map(|x| x.0).collect::<Vec<_>>(),
            "grad": direct.iter().map(|x| x.1).collect::<Vec<_>>(),
            "expansion": expansion,
        }))
    }

    /// Mesh and solve at several refinements and track how the results converge. The
    /// current mesh and solution are left as they were.
    fn study(&mut self, a: StudyArgs, sink: Sink) -> AppResult<f64> {
        a.base.validate()?;
        if a.scales.is_empty() || a.scales.len() > 8 || a.scales.iter().any(|&s| !(s > 0.0 && s <= 10.0)) {
            return Err(AppError::new(400, "1 to 8 refinement scales in (0, 10]"));
        }
        let frame = self.frame();
        let mut scales = a.scales.clone();
        scales.sort_by(|x, y| y.partial_cmp(x).unwrap()); // coarse to fine
        let n = scales.len();
        let t0 = Instant::now();
        let mut levels: Vec<Value> = Vec::new();
        let flag = self.stop.clone();
        let stop = move || flag.load(Ordering::Relaxed);
        let mut outcome = Ok(());
        for (i, &s) in scales.iter().enumerate() {
            let params = MeshParams { h_min: a.base.h_min * s, h_max: a.base.h_max * s, ..a.base.clone() };
            sink(Event::Stage { stage: format!("level {}/{n}: meshing (scale {s})", i + 1) });
            let model = self.model.as_mut().ok_or_else(|| AppError::new(400, "no model loaded"))?;
            let mesh = mesh_bem(&mut self.gmsh, model, &params, &frame)?;
            if let Err(e) = self.check_stop() {
                outcome = Err(e);
                break;
            }
            sink(Event::Stage { stage: format!("level {}/{n}: solving {} panels", i + 1, mesh.tri_body.len()) });
            let model = self.model.as_ref().unwrap();
            let res = match solve::run(&mesh, 0, model, &self.project.labels, a.solve.clone(), sink, &stop) {
                Ok(r) => r,
                Err(e) => {
                    outcome = Err(e.into());
                    break;
                }
            };
            let level = study_level(s, &mesh, &res, a.trap.as_ref());
            sink(Event::Study { level: level.clone() });
            levels.push(level);
        }
        // a stopped study keeps the levels it finished
        if outcome.is_ok() || (self.stop_requested() && !levels.is_empty()) {
            let done = &scales[..levels.len()];
            self.study = Some(json!({
                "levels": levels,
                "quantities": study_quantities(&levels, done),
                "base": a.base,
                "seconds": t0.elapsed().as_secs_f64(),
                "complete": outcome.is_ok(),
            }));
            self.save_study();
        }
        outcome?;
        // gmsh now holds the last study mesh; the app's own mesh copy is unchanged
        Ok(t0.elapsed().as_secs_f64())
    }
}

/// Scalar results of one study level: per electrode the potential, field and curvature at
/// the center (per volt), and the single-ion secular frequencies if a drive was given.
fn study_level(scale: f64, mesh: &SurfaceMesh, res: &SolveResult, trap: Option<&TrapParams>) -> Value {
    let mut q: Vec<(String, String, f64)> = Vec::new();
    for (b, &k) in res.solution.basis.iter().enumerate() {
        let name = &res.electrodes[k].name;
        let l = res.expansions[b].local();
        q.push((format!("{name}: φ₀"), "V/V".into(), l.phi));
        for (c, ax) in ["x", "y", "z"].iter().enumerate() {
            q.push((format!("{name}: ∂φ/∂{ax}"), "V/m per V".into(), l.grad[c]));
        }
        for (i, j, lab) in [(0, 0, "xx"), (1, 1, "yy"), (2, 2, "zz"), (0, 1, "xy"), (0, 2, "xz"), (1, 2, "yz")] {
            q.push((format!("{name}: ∂²φ/∂{lab}"), "V/m² per V".into(), l.hessian[i][j]));
        }
    }
    let mut trap_ok = None;
    if let Some(p) = trap {
        if let Ok(d) = res.drive(p) {
            let a = d.fields.analyze();
            trap_ok = Some(a.confined);
            for (i, m) in a.modes.iter().enumerate() {
                if let Some(f) = m.freq_pseudo {
                    q.push((format!("mode {} frequency", i + 1), "Hz".into(), f));
                }
            }
            for (c, ax) in ["x", "y", "z"].iter().enumerate() {
                q.push((format!("equilibrium {ax}"), "m".into(), a.equilibrium[c]));
            }
        }
    }
    json!({
        "scale": scale,
        "n_panels": mesh.tri_body.len(),
        "mesh_seconds": mesh.stats.seconds,
        "solve_seconds": res.seconds,
        "method": res.solution.stats.method,
        "iterations": res.solution.stats.iterations.iter().max(),
        "trap_confined": trap_ok,
        "values": q.iter().map(|(n, u, v)| json!({"name": n, "unit": u, "value": v})).collect::<Vec<_>>(),
    })
}

/// Per quantity: values over the levels and a Richardson-type extrapolation
/// f(s) ≈ f∞ + C s^p from the three finest levels (s = mesh scale).
fn study_quantities(levels: &[Value], scales: &[f64]) -> Vec<Value> {
    let Some(first) = levels.first() else { return vec![] };
    let names: Vec<(String, String)> = first["values"]
        .as_array()
        .map(|a| a.iter().map(|v| (v["name"].as_str().unwrap_or("").to_string(), v["unit"].as_str().unwrap_or("").to_string())).collect())
        .unwrap_or_default();
    names
        .into_iter()
        .map(|(name, unit)| {
            let vals: Vec<Option<f64>> = levels
                .iter()
                .map(|l| l["values"].as_array().and_then(|a| a.iter().find(|v| v["name"] == name.as_str())).and_then(|v| v["value"].as_f64()))
                .collect();
            let mut out = json!({ "name": name, "unit": unit, "values": vals });
            if let Some(ext) = extrapolate(&vals, scales) {
                out["extrapolated"] = json!(ext.0);
                out["order"] = json!(ext.1);
                let finest = vals.last().copied().flatten().unwrap_or(f64::NAN);
                out["error_estimate"] = json!((finest - ext.0).abs());
            }
            out
        })
        .collect()
}

/// (f∞, p) from the last three levels, if they converge monotonically with 0.25 < p < 8.
fn extrapolate(vals: &[Option<f64>], scales: &[f64]) -> Option<(f64, f64)> {
    let k = vals.len();
    if k < 3 || scales.len() != k {
        return None;
    }
    let (f1, f2, f3) = (vals[k - 3]?, vals[k - 2]?, vals[k - 1]?);
    let (s1, s2, s3) = (scales[k - 3], scales[k - 2], scales[k - 1]);
    let (d12, d23) = (f1 - f2, f2 - f3);
    if d23 == 0.0 {
        return Some((f3, f64::NAN));
    }
    let r = d12 / d23;
    if !(r > 0.0) {
        return None;
    }
    // (s1^p − s2^p)/(s2^p − s3^p) = r, increasing in p: bisection
    let g = |p: f64| (s1.powf(p) - s2.powf(p)) / (s2.powf(p) - s3.powf(p)) - r;
    let (mut lo, mut hi) = (0.25, 8.0);
    if g(lo) * g(hi) > 0.0 {
        return None;
    }
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if g(lo) * g(mid) <= 0.0 {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let p = 0.5 * (lo + hi);
    let c = d23 / (s2.powf(p) - s3.powf(p));
    Some((f3 - c * s3.powf(p), p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extrapolation() {
        // f = 2 + 3 s^1.5
        let scales = [2.0, 1.4, 1.0, 0.7];
        let vals: Vec<Option<f64>> = scales.iter().map(|s: &f64| Some(2.0 + 3.0 * s.powf(1.5))).collect();
        let (f, p) = extrapolate(&vals, &scales).unwrap();
        assert!((f - 2.0).abs() < 1e-9 && (p - 1.5).abs() < 1e-6, "{f} {p}");
        // fewer than three levels, or a missing value: no extrapolation (and no panic)
        assert!(extrapolate(&vals[..2], &scales[..2]).is_none());
        assert!(extrapolate(&[Some(1.0)], &[1.0]).is_none());
        assert!(extrapolate(&[Some(1.0), None, Some(2.0)], &[2.0, 1.0, 0.5]).is_none());
        // a partial study keeps working
        let levels: Vec<Value> = vec![json!({"values": [{"name": "q", "unit": "m", "value": 1.0}]})];
        let q = study_quantities(&levels, &[1.0]);
        assert_eq!(q.len(), 1);
        assert!(q[0].get("extrapolated").is_none());
    }
}
