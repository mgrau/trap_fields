//! HTTP API over [`tf_app::App`]: JSON for state, binary buffers for geometry.
//!
//! Commands run one at a time on the app thread. Long ones (load, mesh, solve) run in
//! the background; while they do, `/state` answers from the last snapshot plus the live
//! job status and progress, so polling never waits on the running command.

use crate::worker::AppWorker;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tf_app::{App, AppError, Event, Reply};

pub struct Running {
    kind: String,
    started: Instant,
    stage: String,
    /// GMRES: (iteration, relative residual per electrode).
    gmres: Vec<(usize, Vec<f64>)>,
    /// Finished levels of a running convergence study.
    study: Vec<Value>,
    /// Stop was requested; re-applied when the command starts (the app clears its flag
    /// then, which would lose a request made while the command was still queued).
    stop_requested: bool,
}

#[derive(Default)]
pub struct Status {
    pub running: Option<Running>,
    /// Last state reported by the app.
    pub snapshot: Value,
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
            v["progress"] = json!({ "gmres": r.gmres, "study": r.study });
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
            }
            Event::Stage { stage } => r.stage = stage.clone(),
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

pub fn router(server: Arc<Server>) -> Router {
    Router::new()
        .route("/state", get(get_state))
        .route("/open", post(|s: S, Json(a): Json<Value>| long(s, "open", a, None)))
        .route("/upload", post(upload))
        .route("/tess", get(|s: S| binary(s, "tess")))
        .route("/transform", post(|s: S, Json(a): Json<Value>| quick(s, "transform", a)))
        .route("/transform/undo", post(|s: S| quick(s, "transform_undo", Value::Null)))
        .route("/transform/reset", post(|s: S| quick(s, "transform_reset", Value::Null)))
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
        .layer(axum::extract::DefaultBodyLimit::max(1 << 30))
        .with_state(server)
}

async fn get_state(State(s): S) -> Json<Value> {
    Json(s.status.lock().unwrap().view())
}

/// Stop the running long command (no-op when idle); the job ends as "stopped".
async fn cancel(State(s): S) -> Json<Value> {
    let mut st = s.status.lock().unwrap();
    if let Some(r) = st.running.as_mut() {
        r.stop_requested = true;
        s.worker.request_stop();
    }
    Json(st.view())
}

fn busy(s: &Server) -> Option<ApiError> {
    s.status.lock().unwrap().running.as_ref().map(|r| ApiError(StatusCode::CONFLICT, format!("busy: {} is running", r.kind)))
}

/// Run a command on the app thread, then refresh the snapshot. State-changing commands
/// are refused while a long command runs; read-only ones wait for it.
async fn run_cmd(s: &Server, cmd: &'static str, args: Value, data: Option<Bytes>) -> ApiResult<Reply> {
    let status = s.status.clone();
    let stop = s.worker.stop_flag();
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
            let snap = app.state();
            status.lock().unwrap().snapshot = snap;
            r
        })
        .await??;
    Ok(reply)
}

async fn quick(State(s): S, cmd: &'static str, args: Value) -> ApiResult<Response> {
    let read_only = matches!(cmd, "solution" | "eval" | "trap" | "slice" | "sweep" | "chain");
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
        st.running = Some(Running { kind: cmd.into(), started: Instant::now(), stage: String::new(), gmres: Vec::new(), study: Vec::new(), stop_requested: false });
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

/// Uploads are short enough to run inline; the reply carries `uploaded_as`.
async fn upload(State(s): S, Query(q): Query<UploadQuery>, body: Bytes) -> ApiResult<Json<Value>> {
    if let Some(e) = busy(&s) {
        return Err(e);
    }
    match run_cmd(&s, "upload", json!({ "name": q.name }), Some(body)).await? {
        Reply::Json(v) => Ok(Json(v)),
        Reply::Binary(_) => Err(ApiError(StatusCode::INTERNAL_SERVER_ERROR, "unexpected reply".into())),
    }
}
