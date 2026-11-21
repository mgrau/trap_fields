//! HTTP API over [`tf_app::App`]: JSON for state, binary buffers for geometry.
//!
//! Commands run one at a time on the app thread. Long ones (load, mesh, solve) run in
//! the background; while they do, `/state` answers from the last snapshot plus the live
//! job status and progress, so polling never waits on the running command.

use crate::worker::AppWorker;
use axum::body::Bytes;
use axum::extract::{Query, Request, State};
use axum::http::{header, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tf_app::{App, AppError, Event, Reply};

pub struct Running {
    /// The command, as started (`kind` becomes the app's job kind, e.g. "load").
    cmd: &'static str,
    kind: String,
    started: Instant,
    stage: String,
    /// GMRES: (iteration, relative residual per electrode).
    gmres: Vec<(usize, Vec<f64>)>,
    /// Finished levels of a running convergence study.
    study: Vec<Value>,
    /// (done, total) items of the current stage, when it counts them.
    step: Option<(usize, usize)>,
    /// The file a load opens (`open`, `upload`), so a forced restart can avoid it.
    target: Option<String>,
    /// Stop was requested; re-applied when the command starts (the app clears its flag
    /// then, which would lose a request made while the command was still queued).
    stop_requested: bool,
}

#[derive(Default)]
pub struct Status {
    pub running: Option<Running>,
    /// Last state reported by the app.
    pub snapshot: Value,
    /// Something to tell the GUI (why the server restarted), reported with the state
    /// until the next long command starts; the GUI shows each one once.
    pub notice: Option<String>,
}

pub struct Server {
    pub worker: AppWorker,
    pub status: Arc<Mutex<Status>>,
}

impl Status {
    fn view(&self) -> Value {
        let mut v = self.snapshot.clone();
        if let Some(r) = &self.running {
            v["job"] = json!({
                "state": "running",
                "kind": r.kind,
                "elapsed": r.started.elapsed().as_secs_f64(),
                "stage": r.stage,
                "stopping": r.stop_requested,
            });
            v["progress"] = json!({ "gmres": r.gmres, "study": r.study, "step": r.step });
        }
        if let Some(n) = &self.notice {
            v["notice"] = json!(n);
        }
        v
    }

    fn apply(&mut self, e: &Event) {
        let Some(r) = self.running.as_mut() else { return };
        match e {
            Event::Start { kind } => {
                r.kind = kind.clone();
                r.stage.clear();
                r.gmres.clear();
                r.study.clear();
                r.step = None;
            }
            Event::Stage { stage } => {
                r.stage = stage.clone();
                r.step = None;
            }
            Event::Progress { done, total } => r.step = Some((*done, *total)),
            Event::Gmres { iteration, residuals } => r.gmres.push((*iteration, residuals.clone())),
            Event::Study { level } => {
                r.study.push(level.clone());
                r.gmres.clear();
            }
        }
    }
}

#[derive(Debug)]
pub struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        ApiError(StatusCode::from_u16(e.status).unwrap_or(StatusCode::BAD_REQUEST), e.message)
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
    }
}

type ApiResult<T> = Result<T, ApiError>;
type S = State<Arc<Server>>;

/// Host name (without port) of a Host header or an Origin URL is a loopback name.
fn loopback(hostport: &str) -> bool {
    let h = hostport.trim_start_matches("http://").trim_start_matches("https://");
    let name = if let Some(rest) = h.strip_prefix('[') { rest.split(']').next().map(|v| format!("[{v}]")).unwrap_or_default() } else { h.split(':').next().unwrap_or("").to_string() };
    matches!(name.as_str(), "127.0.0.1" | "localhost" | "[::1]")
}

/// Only the GUI may use the API. The server listens on loopback, but any web page in the
/// user's browser can send requests to it: require a loopback Host (no DNS rebinding), no
/// foreign Origin, and on state-changing requests the X-Trapfields header, which a
/// cross-site page cannot send without a CORS preflight (and none is answered).
async fn guard(req: Request, next: Next) -> Response {
    let h = req.headers();
    let host_ok = h.get(header::HOST).and_then(|v| v.to_str().ok()).is_some_and(loopback);
    let origin_ok = h.get(header::ORIGIN).is_none_or(|v| v.to_str().is_ok_and(|o| o != "null" && loopback(o)));
    let marked = req.method() == Method::GET || h.contains_key("x-trapfields");
    if host_ok && origin_ok && marked {
        next.run(req).await
    } else {
        ApiError(StatusCode::FORBIDDEN, "request not from the trapfields GUI".into()).into_response()
    }
}

pub fn router(server: Arc<Server>) -> Router {
    Router::new()
        .route("/state", get(get_state))
        .route("/open", post(|s: S, Json(a): Json<Value>| long(s, "open", a, None)))
        .route(
            "/upload",
            post(|s: S, Query(q): Query<UploadQuery>, body: Bytes| long(s, "upload", json!({ "name": q.name }), Some(body)))
                .layer(axum::extract::DefaultBodyLimit::max(1 << 30)),
        )
        .route("/tess", get(|s: S| binary(s, "tess")))
        .route("/transform", post(|s: S, Json(a): Json<Value>| quick(s, "transform", a)))
        .route("/transform/undo", post(|s: S| quick(s, "transform_undo", Value::Null)))
        .route("/transform/reset", post(|s: S| quick(s, "transform_reset", Value::Null)))
        .route("/features/faces", post(|s: S, Json(a): Json<Value>| quick(s, "feature_faces", a)))
        .route("/features/remove", post(|s: S, Json(a): Json<Value>| long(s, "remove_features", a, None)))
        .route("/features/undo", post(|s: S| long(s, "features_undo", Value::Null, None)))
        .route("/features/reset", post(|s: S| long(s, "features_reset", Value::Null, None)))
        .route("/labels", put(|s: S, Json(a): Json<Value>| quick(s, "labels", a)))
        .route("/mesh", post(|s: S, Json(a): Json<Value>| long(s, "mesh", a, None)).get(|s: S| binary(s, "mesh_data")))
        .route("/solve", post(|s: S, Json(a): Json<Value>| long(s, "solve", a, None)).get(|s: S| quick(s, "solution", Value::Null)))
        .route("/solve/sigma", get(|s: S| binary(s, "sigma")))
        .route("/solve/eval", post(|s: S, Json(a): Json<Value>| quick(s, "eval", a)))
        .route("/trap", post(|s: S, Json(a): Json<Value>| quick(s, "trap", a)))
        .route("/trap/slice", post(|s: S, Json(a): Json<Value>| quick(s, "slice", a)))
        .route("/sweep", post(|s: S, Json(a): Json<Value>| quick(s, "sweep", a)))
        .route("/chain", post(|s: S, Json(a): Json<Value>| quick(s, "chain", a)))
        .route("/study", post(|s: S, Json(a): Json<Value>| long(s, "study", a, None)))
        .route("/cancel", post(cancel))
        .route("/results", get(|s: S| quick(s, "results", Value::Null)))
        .route("/export", get(|s: S| binary(s, "export_project")))
        .route("/import", post(|s: S, body: Bytes| long(s, "import_project", Value::Null, Some(body))).layer(axum::extract::DefaultBodyLimit::max(1 << 30)))
        .layer(axum::extract::DefaultBodyLimit::max(64 << 20))
        .layer(middleware::from_fn(guard))
        .with_state(server)
}

async fn get_state(State(s): S) -> Json<Value> {
    Json(s.status.lock().unwrap().view())
}

/// How long a stop may take to land before the server restarts itself to force it. A
/// single gmsh call (the OpenCASCADE import of a big STEP file, one meshing pass) cannot
/// be interrupted and can run for minutes, and gmsh lives in this process.
const STOP_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// Jobs that spend their time in gmsh calls, and leave nothing half-written when the
/// process is replaced (files are written atomically, the project only at the end).
const FORCEABLE: [&str; 5] = ["startup", "open", "upload", "import_project", "mesh"];

/// Environment variable carrying a forced stop to the restarted server: JSON
/// {kind, skip_reopen}, where skip_reopen is the STEP file not to re-open ("*": any).
pub const STOPPED_ENV: &str = "TRAPFIELDS_STOPPED";

/// Replace this process with a fresh copy of the server (same arguments, minus --open),
/// which ends the stuck gmsh call. Only returns if that fails.
#[cfg(unix)]
fn restart_server(kind: &str, skip_reopen: Option<&str>) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    tracing::warn!("stop of {kind} did not land within {STOP_GRACE:?}: restarting the server");
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => return e,
    };
    let args = std::env::args_os().skip(1).filter(|a| a != "--open");
    // the listening socket is close-on-exec: the new process can bind the same port
    std::process::Command::new(exe).args(args).env(STOPPED_ENV, json!({ "kind": kind, "skip_reopen": skip_reopen }).to_string()).exec()
}

#[cfg(not(unix))]
fn restart_server(_kind: &str, _skip_reopen: Option<&str>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Unsupported, "restarting is only supported on Unix")
}

/// Stop the running long command (no-op when idle); the job ends as "stopped".
async fn cancel(State(s): S) -> Json<Value> {
    let mut st = s.status.lock().unwrap();
    if let Some(r) = st.running.as_mut() {
        let first = !r.stop_requested;
        r.stop_requested = true;
        s.worker.request_stop();
        if first && FORCEABLE.contains(&r.cmd) {
            // the restarted server re-opens the project's STEP file unless that is what hung
            let started = r.started;
            // (an imported bundle is on disk by the time its STEP file is opened)
            let skip = if matches!(r.cmd, "startup" | "import_project") { Some("*".to_string()) } else { r.target.clone() };
            let status = s.status.clone();
            std::thread::spawn(move || {
                std::thread::sleep(STOP_GRACE);
                let st = status.lock().unwrap();
                let Some(r) = st.running.as_ref().filter(|r| r.started == started) else { return };
                let e = restart_server(&r.kind, skip.as_deref());
                tracing::error!("could not restart the server to stop {}: {e}", r.kind);
            });
        }
    }
    Json(st.view())
}

fn busy(s: &Server) -> Option<ApiError> {
    s.status.lock().unwrap().running.as_ref().map(|r| ApiError(StatusCode::CONFLICT, format!("busy: {} is running", r.kind)))
}

/// Commands that change nothing the state reports (no snapshot refresh afterwards).
const READ_ONLY: [&str; 12] = ["state", "feature_faces", "solution", "eval", "slice", "sweep", "chain", "results", "tess", "mesh_data", "sigma", "export_project"];

/// Run a command on the app thread, then refresh the snapshot. State-changing commands
/// are refused while a long command runs; read-only ones wait for it.
async fn run_cmd(s: &Server, cmd: &'static str, args: Value, data: Option<Bytes>) -> ApiResult<Reply> {
    let status = s.status.clone();
    let stop = s.worker.stop_flag();
    let refresh = !READ_ONLY.contains(&cmd);
    let reply = s
        .worker
        .run(move |app: &mut App| {
            let sink = |e: Event| {
                let mut st = status.lock().unwrap();
                st.apply(&e);
                if matches!(e, Event::Start { .. }) && st.running.as_ref().is_some_and(|r| r.stop_requested) {
                    stop.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            };
            let r = app.command(cmd, args, data.as_deref(), &sink);
            if refresh {
                let snap = app.state();
                status.lock().unwrap().snapshot = snap;
            }
            r
        })
        .await;
    match reply {
        Ok(r) => Ok(r?),
        Err(e) => {
            // the command panicked: record a failed job so the GUI does not show the last one
            if let Ok(snap) = s.worker.run(|app: &mut App| {
                app.record_panic();
                app.state()
            })
            .await
            {
                s.status.lock().unwrap().snapshot = snap;
            }
            Err(e.into())
        }
    }
}

async fn quick(State(s): S, cmd: &'static str, args: Value) -> ApiResult<Response> {
    let read_only = matches!(cmd, "feature_faces" | "solution" | "eval" | "trap" | "slice" | "sweep" | "chain" | "results");
    if !read_only {
        if let Some(e) = busy(&s) {
            return Err(e);
        }
    }
    match run_cmd(&s, cmd, args, None).await? {
        // state-changing commands reply with the full (refreshed) state
        Reply::Json(_) if !read_only => Ok(Json(s.status.lock().unwrap().view()).into_response()),
        Reply::Json(v) => Ok(Json(v).into_response()),
        Reply::Binary(b) => Ok(([(header::CONTENT_TYPE, "application/octet-stream")], b).into_response()),
    }
}

async fn binary(State(s): S, cmd: &'static str) -> ApiResult<Response> {
    match run_cmd(&s, cmd, Value::Null, None).await? {
        Reply::Binary(b) => Ok(([(header::CONTENT_TYPE, "application/octet-stream")], b).into_response()),
        Reply::Json(v) => Ok(Json(v).into_response()),
    }
}

/// Start a long command in the background and return the state with the job running.
async fn long(State(s): S, cmd: &'static str, args: Value, data: Option<Bytes>) -> ApiResult<Json<Value>> {
    start_long(&s, cmd, args, data)?;
    Ok(Json(s.status.lock().unwrap().view()))
}

pub fn start_long(s: &Arc<Server>, cmd: &'static str, args: Value, data: Option<Bytes>) -> ApiResult<()> {
    {
        let mut st = s.status.lock().unwrap();
        if let Some(r) = &st.running {
            return Err(ApiError(StatusCode::CONFLICT, format!("busy: {} is running", r.kind)));
        }
        st.notice = None;
        let target = args.get("file").or_else(|| args.get("name")).and_then(|v| v.as_str()).map(String::from);
        st.running = Some(Running { cmd, target, kind: cmd.into(), started: Instant::now(), stage: String::new(), gmres: Vec::new(), study: Vec::new(), step: None, stop_requested: false });
    }
    let s2 = s.clone();
    tokio::spawn(async move {
        let r = run_cmd(&s2, cmd, args, data).await;
        if let Err(ApiError(_, msg)) = &r {
            tracing::warn!("{cmd} failed: {msg}");
        }
        s2.status.lock().unwrap().running = None;
    });
    Ok(())
}

#[derive(Deserialize)]
struct UploadQuery {
    name: String,
}
