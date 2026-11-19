//! HTTP API. JSON for state, compact little-endian binary buffers for geometry.

use crate::project::{step_files, Label, Project, Role};
use crate::solve::{self, SolveParams, SolveResult, TrapParams};
use crate::worker::GmshWorker;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tf_geom::{load_step, mesh_bem, metrics, tessellate, MeshParams, MeshStats, Model, Rigid, SurfaceMesh, Tessellation, TransformOp};
use tokio::sync::Mutex;

pub struct AppState {
    pub dir: PathBuf,
    pub worker: GmshWorker,
    pub inner: Mutex<Inner>,
}

#[derive(Default)]
pub struct Inner {
    pub project: Project,
    pub model: Option<Model>,
    pub tess: Option<Arc<Tessellation>>,
    pub tess_version: u64,
    pub mesh: Option<Arc<SurfaceMesh>>,
    pub mesh_version: u64,
    pub job: Job,
    /// Progress of the running job, if it reports any.
    pub stage: Option<&'static str>,
    pub solution: Option<Arc<SolveResult>>,
    pub solution_version: u64,
    pub(crate) cache: Option<GeomCache>,
}

#[derive(Default, Clone)]
pub enum Job {
    #[default]
    Idle,
    Running { kind: &'static str, started: Instant },
    Done { kind: &'static str, seconds: f64 },
    Failed { kind: &'static str, error: String },
}

impl Job {
    fn is_running(&self) -> bool {
        matches!(self, Job::Running { .. })
    }
    fn view(&self) -> serde_json::Value {
        match self {
            Job::Idle => json!({"state": "idle"}),
            Job::Running { kind, started } => json!({"state": "running", "kind": kind, "elapsed": started.elapsed().as_secs_f64()}),
            Job::Done { kind, seconds } => json!({"state": "done", "kind": kind, "seconds": seconds}),
            Job::Failed { kind, error } => json!({"state": "failed", "kind": kind, "error": error}),
        }
    }
}

/// Derived geometry in the current (transformed) frame.
#[derive(Clone, Serialize)]
pub(crate) struct GeomCache {
    body_bbox: Vec<[f64; 6]>,
    /// Closest distance (m) and point from the frame origin to each body's surface.
    closest: Vec<Option<(f64, [f64; 3])>>,
    bbox: [f64; 6],
}

fn compute_cache(model: &Model, tess: &Tessellation, frame: &Rigid) -> GeomCache {
    let n = model.bodies.len();
    let mut pos = tess.positions.clone();
    frame.apply_flat(&mut pos);
    let mut body_bbox = vec![[f64::INFINITY, f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY]; n];
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
    let mut bbox = [f64::INFINITY, f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for b in &body_bbox {
        for k in 0..3 {
            bbox[k] = bbox[k].min(b[k]);
            bbox[k + 3] = bbox[k + 3].max(b[k + 3]);
        }
    }
    let closest = metrics::closest_per_group([0.0; 3], &pos, &tess.triangles, &tess.tri_body, n);
    GeomCache { body_bbox, closest, bbox }
}

impl Inner {
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

    fn frame(&self) -> Rigid {
        Rigid::from_ops(&self.project.transforms)
    }

    fn refresh_cache(&mut self) {
        self.cache = match (&self.model, &self.tess) {
            (Some(m), Some(t)) => Some(compute_cache(m, t, &self.frame())),
            _ => None,
        };
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
        Some(MeshParams {
            center: [0.0; 3],
            h_min,
            h_max: r2(diag / 25.0).max(h_min),
            dist_min: r2(near),
            dist_max: r2(10.0 * near),
            min_circle_points: 16,
        })
    }
}

pub struct ApiError(StatusCode, String);

impl ApiError {
    pub fn message(&self) -> &str {
        &self.1
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}

impl<E: std::fmt::Display> From<E> for ApiError {
    fn from(e: E) -> Self {
        ApiError(StatusCode::BAD_REQUEST, e.to_string())
    }
}

fn err(code: StatusCode, msg: impl Into<String>) -> ApiError {
    ApiError(code, msg.into())
}

type ApiResult<T> = Result<T, ApiError>;
type S = State<Arc<AppState>>;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/state", get(get_state))
        .route("/open", post(open))
        .route("/upload", post(upload))
        .route("/tess", get(get_tess))
        .route("/transform", post(transform))
        .route("/transform/undo", post(transform_undo))
        .route("/transform/reset", post(transform_reset))
        .route("/labels", put(put_labels))
        .route("/mesh", post(start_mesh).get(get_mesh))
        .route("/solve", post(start_solve).get(get_solve))
        .route("/solve/sigma", get(get_sigma))
        .route("/solve/eval", post(eval))
        .route("/trap", post(trap))
        .route("/trap/slice", post(trap_slice))
        .layer(axum::extract::DefaultBodyLimit::max(1 << 30))
        .with_state(state)
}

async fn state_json(s: &AppState) -> serde_json::Value {
    let inner = s.inner.lock().await;
    let bodies: Vec<serde_json::Value> = inner
        .model
        .iter()
        .flat_map(|m| m.bodies.iter())
        .map(|b| {
            let c = inner.cache.as_ref();
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
                "label": inner.project.labels.get(&b.name),
            })
        })
        .collect();
    let frame = inner.frame();
    let mut job = inner.job.view();
    if let (true, Some(st)) = (inner.job.is_running(), inner.stage) {
        job["stage"] = json!(st);
    }
    json!({
        "project_dir": s.dir.display().to_string(),
        "files": step_files(&s.dir),
        "project": inner.project,
        "model": inner.model.as_ref().map(|m| json!({
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
        "bbox": inner.cache.as_ref().map(|c| c.bbox),
        "frame": {"r": frame.r, "t": frame.t},
        "tess": inner.tess.as_ref().map(|t| json!({
            "version": inner.tess_version,
            "n_triangles": t.tri_body.len(),
            "failed_faces": t.failed_faces,
            "seconds": t.seconds,
        })),
        "mesh": inner.mesh.as_ref().map(|m| json!({
            "version": inner.mesh_version,
            "params": m.params,
            "stats": m.stats,
        })),
        "mesh_defaults": inner.mesh_defaults(),
        "solve": inner.solution.as_ref().map(|r| json!({
            "version": inner.solution_version,
            "mesh_version": r.mesh_version,
            "n_panels": r.problem.panels.len(),
            "seconds": r.seconds,
            "stale": inner.solution_stale(),
        })),
        "job": job,
    })
}

async fn get_state(State(s): S) -> Json<serde_json::Value> {
    Json(state_json(&s).await)
}

#[derive(Deserialize)]
struct OpenReq {
    file: String,
}

fn project_file(dir: &Path, name: &str) -> ApiResult<PathBuf> {
    let base = Path::new(name).file_name().ok_or_else(|| err(StatusCode::BAD_REQUEST, "invalid file name"))?;
    let lower = base.to_string_lossy().to_lowercase();
    if !(lower.ends_with(".step") || lower.ends_with(".stp")) {
        return Err(err(StatusCode::BAD_REQUEST, "only .step / .stp files are supported"));
    }
    Ok(dir.join(base))
}

pub async fn open_file(s: &Arc<AppState>, file: &str) -> ApiResult<()> {
    let path = project_file(&s.dir, file)?;
    if !path.exists() {
        return Err(err(StatusCode::NOT_FOUND, format!("{file} not found in project directory")));
    }
    {
        let mut inner = s.inner.lock().await;
        if inner.job.is_running() {
            return Err(err(StatusCode::CONFLICT, "a job is running"));
        }
        inner.job = Job::Running { kind: "load", started: Instant::now() };
    }
    let file = path.file_name().unwrap().to_string_lossy().into_owned();
    let result = s
        .worker
        .run(move |ctx| -> tf_geom::Result<(Model, Tessellation)> {
            ctx.model = None;
            let mut m = load_step(&mut ctx.gmsh, &path)?;
            let bb = ctx.gmsh.bounding_box(-1, -1)?;
            let diag = ((0..3).map(|k| (bb[k + 3] - bb[k]).powi(2)).sum::<f64>()).sqrt();
            let t = tessellate(&mut ctx.gmsh, &mut m, diag / 100.0)?;
            ctx.model = Some(m.clone());
            Ok((m, t))
        })
        .await;
    let mut inner = s.inner.lock().await;
    let (model, tess) = match result.map_err(|e| e.to_string()).and_then(|r| r.map_err(|e| e.to_string())) {
        Ok(v) => v,
        Err(e) => {
            inner.job = Job::Failed { kind: "load", error: e.clone() };
            return Err(err(StatusCode::BAD_REQUEST, e));
        }
    };
    if inner.project.step_file.as_deref() != Some(file.as_str()) {
        inner.project.transforms.clear();
        inner.project.mesh_params = None;
    }
    inner.project.step_file = Some(file);
    for b in &model.bodies {
        inner
            .project
            .labels
            .entry(b.name.clone())
            .or_insert_with(|| Label { electrode: b.name.clone(), role: Role::Unassigned });
    }
    inner.job = Job::Done { kind: "load", seconds: model.load_seconds + tess.seconds };
    inner.model = Some(model);
    inner.tess = Some(Arc::new(tess));
    inner.tess_version += 1;
    inner.drop_mesh();
    inner.refresh_cache();
    inner.project.save(&s.dir)?;
    Ok(())
}

async fn open(State(s): S, Json(req): Json<OpenReq>) -> ApiResult<Json<serde_json::Value>> {
    open_file(&s, &req.file).await?;
    Ok(Json(state_json(&s).await))
}

#[derive(Deserialize)]
struct UploadQuery {
    name: String,
}

/// Save an uploaded STEP file into the project directory (never overwriting a
/// different existing file) and open it.
async fn upload(State(s): S, Query(q): Query<UploadQuery>, body: Bytes) -> ApiResult<Json<serde_json::Value>> {
    let mut path = project_file(&s.dir, &q.name)?;
    if !body.starts_with(b"ISO-10303-21") {
        return Err(err(StatusCode::BAD_REQUEST, "not a STEP file (missing ISO-10303-21 header)"));
    }
    if path.exists() && std::fs::read(&path)? != body.as_ref() {
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let ext = path.extension().unwrap().to_string_lossy().into_owned();
        let mut n = 2;
        while path.exists() {
            path = s.dir.join(format!("{stem}-{n}.{ext}"));
            n += 1;
        }
    }
    if !path.exists() {
        std::fs::write(&path, &body)?;
    }
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    open_file(&s, &name).await?;
    let mut v = state_json(&s).await;
    v["uploaded_as"] = json!(name);
    Ok(Json(v))
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

fn binary(buf: Vec<u8>) -> Response {
    ([(header::CONTENT_TYPE, "application/octet-stream")], buf).into_response()
}

/// TFT1 | nv nt ne np | f32 pos[3nv] | u32 tri[3nt] | u32 tri_body[nt] | u32 tri_face[nt]
///      | f32 edges[6ne] | u32 edge_body[ne] | f32 points[3np] | u32 point_body[np]
/// Coordinates in meters, transformed frame.
async fn get_tess(State(s): S) -> ApiResult<Response> {
    let (tess, frame) = {
        let inner = s.inner.lock().await;
        (inner.tess.clone().ok_or_else(|| err(StatusCode::NOT_FOUND, "no model loaded"))?, inner.frame())
    };
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
    Ok(binary(buf))
}

async fn edit_transforms(s: &AppState, f: impl FnOnce(&mut Vec<TransformOp>)) -> ApiResult<()> {
    let mut inner = s.inner.lock().await;
    if inner.job.is_running() {
        return Err(err(StatusCode::CONFLICT, "a job is running"));
    }
    if inner.model.is_none() {
        return Err(err(StatusCode::BAD_REQUEST, "no model loaded"));
    }
    f(&mut inner.project.transforms);
    // a mesh (and its solution) lives in the frame it was generated in
    inner.drop_mesh();
    inner.refresh_cache();
    inner.project.save(&s.dir)?;
    Ok(())
}

async fn transform(State(s): S, Json(op): Json<TransformOp>) -> ApiResult<Json<serde_json::Value>> {
    edit_transforms(&s, |t| t.push(op)).await?;
    Ok(Json(state_json(&s).await))
}

async fn transform_undo(State(s): S) -> ApiResult<Json<serde_json::Value>> {
    edit_transforms(&s, |t| {
        t.pop();
    })
    .await?;
    Ok(Json(state_json(&s).await))
}

async fn transform_reset(State(s): S) -> ApiResult<Json<serde_json::Value>> {
    edit_transforms(&s, |t| t.clear()).await?;
    Ok(Json(state_json(&s).await))
}

async fn put_labels(State(s): S, Json(labels): Json<BTreeMap<String, Label>>) -> ApiResult<Json<serde_json::Value>> {
    {
        let mut inner = s.inner.lock().await;
        for (k, v) in labels {
            let electrode = v.electrode.trim().to_string();
            if electrode.is_empty() {
                return Err(err(StatusCode::BAD_REQUEST, format!("electrode name for '{k}' is empty")));
            }
            inner.project.labels.insert(k, Label { electrode, role: v.role });
        }
        inner.project.save(&s.dir)?;
    }
    Ok(Json(state_json(&s).await))
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

fn encode_mesh(m: &SurfaceMesh) -> Vec<u8> {
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

fn save_mesh(dir: &Path, inner: &Inner, m: &SurfaceMesh) -> anyhow::Result<()> {
    let out = dir.join("mesh");
    std::fs::create_dir_all(&out)?;
    let model = inner.model.as_ref().ok_or_else(|| anyhow::anyhow!("no model"))?;
    let header = MeshFileHeader {
        format: "TFM1 little-endian: magic, u32 n_nodes, u32 n_tri, f32 nodes[3n] (m), u32 tri[3t], u32 tri_body[t], u32 tri_face[t]",
        step_file: inner.project.step_file.as_deref(),
        transforms: &inner.project.transforms,
        bodies: model.bodies.iter().map(|b| b.name.as_str()).collect(),
        labels: model.bodies.iter().map(|b| inner.project.labels.get(&b.name)).collect(),
        params: &m.params,
        stats: &m.stats,
    };
    std::fs::write(out.join("surface_mesh.json"), serde_json::to_string_pretty(&header)?)?;
    std::fs::write(out.join("surface_mesh.bin"), encode_mesh(m))?;
    // full-precision node coordinates, used to restore the mesh exactly on restart
    let raw: Vec<u8> = m.nodes.iter().flat_map(|x| x.to_le_bytes()).collect();
    std::fs::write(out.join("surface_mesh_nodes.f64"), raw)?;
    Ok(())
}

#[derive(Deserialize)]
struct SavedMeshHeader {
    step_file: Option<String>,
    transforms: Vec<TransformOp>,
    bodies: Vec<String>,
    params: MeshParams,
    stats: MeshStats,
}

/// The saved mesh, if it belongs to the current STEP file, frame and bodies.
fn load_mesh(dir: &Path, inner: &Inner) -> anyhow::Result<Option<SurfaceMesh>> {
    let d = dir.join("mesh");
    let (Ok(hdr), Ok(bin)) = (std::fs::read_to_string(d.join("surface_mesh.json")), std::fs::read(d.join("surface_mesh.bin"))) else {
        return Ok(None);
    };
    let h: SavedMeshHeader = serde_json::from_str(&hdr)?;
    let model = inner.model.as_ref().ok_or_else(|| anyhow::anyhow!("no model"))?;
    let names: Vec<&str> = model.bodies.iter().map(|b| b.name.as_str()).collect();
    if h.step_file != inner.project.step_file || h.transforms != inner.project.transforms || h.bodies != names {
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
    anyhow::ensure!(triangles.iter().all(|&i| (i as usize) < nn) && tri_body.iter().all(|&b| (b as usize) < names.len()), "inconsistent mesh file");
    Ok(Some(SurfaceMesh { nodes, triangles, tri_body, tri_face, params: h.params, stats: h.stats }))
}

/// After the STEP file is (re)opened at startup: restore the saved mesh and solution
/// when they match the current project.
pub async fn restore_outputs(s: &Arc<AppState>) {
    let (mesh, version) = {
        let mut inner = s.inner.lock().await;
        let mesh = match load_mesh(&s.dir, &inner) {
            Ok(Some(m)) => Arc::new(m),
            Ok(None) => return,
            Err(e) => {
                tracing::warn!("saved mesh not restored: {e:#}");
                return;
            }
        };
        inner.mesh = Some(mesh.clone());
        inner.mesh_version += 1;
        tracing::info!("restored mesh ({} panels)", mesh.tri_body.len());
        (mesh, inner.mesh_version)
    };
    let (model, project) = {
        let inner = s.inner.lock().await;
        (inner.model.clone(), inner.project.clone())
    };
    let Some(model) = model else { return };
    let dir = s.dir.clone();
    let r = tokio::task::spawn_blocking(move || solve::load(&dir, &mesh, version, &model, &project)).await;
    let mut inner = s.inner.lock().await;
    match r {
        Ok(Ok(Some(res))) if inner.mesh_version == version => {
            tracing::info!("restored solution ({} basis solutions)", res.solution.u.len());
            inner.solution_version += 1;
            inner.solution = Some(Arc::new(res));
        }
        Ok(Err(e)) => tracing::warn!("saved solution not restored: {e:#}"),
        _ => {}
    }
}

async fn start_mesh(State(s): S, Json(params): Json<MeshParams>) -> ApiResult<Json<serde_json::Value>> {
    params.validate()?;
    let frame = {
        let mut inner = s.inner.lock().await;
        if inner.job.is_running() {
            return Err(err(StatusCode::CONFLICT, "a job is already running"));
        }
        if inner.model.is_none() {
            return Err(err(StatusCode::BAD_REQUEST, "no model loaded"));
        }
        inner.job = Job::Running { kind: "mesh", started: Instant::now() };
        inner.project.mesh_params = Some(params.clone());
        inner.project.save(&s.dir)?;
        inner.frame()
    };
    let s2 = s.clone();
    tokio::spawn(async move {
        let r = s2
            .worker
            .run(move |ctx| match ctx.model.as_mut() {
                Some(m) => mesh_bem(&mut ctx.gmsh, m, &params, &frame).map_err(|e| e.to_string()),
                None => Err("no model loaded".to_string()),
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);
        let mut inner = s2.inner.lock().await;
        match r {
            Ok(mesh) => {
                if let Err(e) = save_mesh(&s2.dir, &inner, &mesh) {
                    tracing::warn!("could not save mesh: {e}");
                }
                inner.job = Job::Done { kind: "mesh", seconds: mesh.stats.seconds };
                inner.mesh = Some(Arc::new(mesh));
                inner.mesh_version += 1;
                inner.solution = None;
            }
            Err(e) => inner.job = Job::Failed { kind: "mesh", error: e },
        }
    });
    Ok(Json(state_json(&s).await))
}

async fn get_mesh(State(s): S) -> ApiResult<Response> {
    let mesh = s.inner.lock().await.mesh.clone().ok_or_else(|| err(StatusCode::NOT_FOUND, "no mesh"))?;
    Ok(binary(encode_mesh(&mesh)))
}

async fn start_solve(State(s): S, Json(params): Json<SolveParams>) -> ApiResult<Json<serde_json::Value>> {
    let (mesh, mesh_version, model, labels) = {
        let mut inner = s.inner.lock().await;
        if inner.job.is_running() {
            return Err(err(StatusCode::CONFLICT, "a job is already running"));
        }
        let mesh = inner.mesh.clone().ok_or_else(|| err(StatusCode::BAD_REQUEST, "generate a surface mesh first"))?;
        let model = inner.model.clone().ok_or_else(|| err(StatusCode::BAD_REQUEST, "no model loaded"))?;
        inner.job = Job::Running { kind: "solve", started: Instant::now() };
        inner.stage = Some("starting");
        inner.project.solve_params = Some(params.clone());
        inner.project.save(&s.dir)?;
        (mesh, inner.mesh_version, model, inner.project.labels.clone())
    };
    let s2 = s.clone();
    tokio::task::spawn_blocking(move || {
        let progress = |st: &'static str| s2.inner.blocking_lock().stage = Some(st);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| solve::run(&mesh, mesh_version, &model, &labels, params, &progress)));
        let mut inner = s2.inner.blocking_lock();
        inner.stage = None;
        inner.job = match r {
            Ok(Ok(res)) if inner.mesh_version == res.mesh_version => {
                inner.solution_version += 1;
                if let Err(e) = solve::save(&s2.dir, &res, inner.solution_version, &inner.project, &mesh) {
                    tracing::warn!("could not save solution: {e}");
                }
                tracing::info!("solve: {} panels, {:?}, total {:.2}s", res.problem.panels.len(), res.timings, res.seconds);
                let seconds = res.seconds;
                inner.solution = Some(Arc::new(res));
                Job::Done { kind: "solve", seconds }
            }
            Ok(Ok(_)) => Job::Failed { kind: "solve", error: "the mesh changed during the solve".into() },
            Ok(Err(e)) => Job::Failed { kind: "solve", error: format!("{e:#}") },
            Err(_) => Job::Failed { kind: "solve", error: "the solver panicked (see server log)".into() },
        };
    });
    Ok(Json(state_json(&s).await))
}

async fn get_solve(State(s): S) -> ApiResult<Json<serde_json::Value>> {
    let inner = s.inner.lock().await;
    let r = inner.solution.as_ref().ok_or_else(|| err(StatusCode::NOT_FOUND, "no solution"))?;
    let mut v = r.summary(inner.solution_version);
    v["stale"] = json!(inner.solution_stale());
    Ok(Json(v))
}

/// TFS1 surface charge densities, see `solve::encode_sigma`.
async fn get_sigma(State(s): S) -> ApiResult<Response> {
    let r = s.inner.lock().await.solution.clone().ok_or_else(|| err(StatusCode::NOT_FOUND, "no solution"))?;
    Ok(binary(solve::encode_sigma(&r)))
}

#[derive(Deserialize)]
struct EvalReq {
    /// Trap-frame points (m).
    points: Vec<[f64; 3]>,
    /// Electrode voltages (V); electrodes not listed are at 0 V.
    voltages: BTreeMap<String, f64>,
}

const MAX_EVAL_POINTS: usize = 200_000;

/// Potential and gradient at points for a set of electrode voltages, by direct
/// integration over the surface charges and (inside r_min) from the expansion.
async fn eval(State(s): S, Json(req): Json<EvalReq>) -> ApiResult<Json<serde_json::Value>> {
    if req.points.len() > MAX_EVAL_POINTS {
        return Err(err(StatusCode::BAD_REQUEST, format!("at most {MAX_EVAL_POINTS} points per request")));
    }
    let r = s.inner.lock().await.solution.clone().ok_or_else(|| err(StatusCode::NOT_FOUND, "no solution"))?;
    let out = tokio::task::spawn_blocking(move || -> anyhow::Result<serde_json::Value> {
        let (u, exp) = r.combine(&req.voltages)?;
        let direct = tf_bem::evaluate(&r.problem.panels, &u, &req.points);
        let expansion: Option<Vec<serde_json::Value>> = exp.map(|e| {
            req.points
                .iter()
                .map(|&p| {
                    let d = ((p[0] - e.center[0]).powi(2) + (p[1] - e.center[1]).powi(2) + (p[2] - e.center[2]).powi(2)).sqrt();
                    if d < r.r_min {
                        let (phi, grad) = e.potential_grad(p);
                        json!({"phi": phi, "grad": grad})
                    } else {
                        serde_json::Value::Null
                    }
                })
                .collect()
        });
        Ok(json!({
            "phi": direct.iter().map(|x| x.0).collect::<Vec<_>>(),
            "grad": direct.iter().map(|x| x.1).collect::<Vec<_>>(),
            "expansion": expansion,
        }))
    })
    .await
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))??;
    Ok(Json(out))
}

async fn trap(State(s): S, Json(p): Json<TrapParams>) -> ApiResult<Json<serde_json::Value>> {
    let r = {
        let mut inner = s.inner.lock().await;
        inner.project.trap_params = Some(TrapParams { depth: false, ..p.clone() });
        inner.project.save(&s.dir)?;
        inner.solution.clone().ok_or_else(|| err(StatusCode::NOT_FOUND, "solve first"))?
    };
    let v = tokio::task::spawn_blocking(move || r.analyze(&p))
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))??;
    Ok(Json(v))
}

#[derive(Deserialize)]
struct SliceReq {
    params: TrapParams,
    /// 0, 1, 2: plane normal along x, y, z.
    axis: usize,
    center: [f64; 3],
    half: f64,
    n: usize,
}

async fn trap_slice(State(s): S, Json(q): Json<SliceReq>) -> ApiResult<Json<serde_json::Value>> {
    let r = s.inner.lock().await.solution.clone().ok_or_else(|| err(StatusCode::NOT_FOUND, "solve first"))?;
    let v = tokio::task::spawn_blocking(move || r.slice(&q.params, q.axis, q.center, q.half, q.n))
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))??;
    Ok(Json(v))
}
