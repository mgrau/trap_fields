//! `trapfields` — RF Paul trap field simulator.

mod api;
mod worker;

use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::services::{ServeDir, ServeFile};

#[derive(Parser)]
#[command(name = "trapfields", version, about = "RF Paul trap field simulator")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Start the local GUI server for a project directory.
    Gui {
        /// Project directory (holds STEP files, trapfields.json, mesh/ outputs).
        #[arg(default_value = ".")]
        project: PathBuf,
        #[arg(long, default_value_t = 8765)]
        port: u16,
        /// Open the GUI in the default browser.
        #[arg(long)]
        open: bool,
        /// Built frontend directory (default: frontend/dist in the source tree).
        #[arg(long)]
        static_dir: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    match Cli::parse().cmd {
        Cmd::Gui { project, port, open, static_dir } => gui(project, port, open, static_dir).await,
    }
}

async fn gui(project: PathBuf, port: u16, open: bool, static_dir: Option<PathBuf>) -> anyhow::Result<()> {
    let dir = project.canonicalize().map_err(|e| anyhow::anyhow!("project dir {}: {e}", project.display()))?;
    let state = Arc::new(api::Server {
        worker: worker::AppWorker::spawn(dir.clone())?,
        status: Default::default(),
    });
    // initial snapshot, then re-open the project's STEP file (and restore its mesh and
    // solution) in the background
    let snap = state.worker.run(|app| app.state()).await?;
    state.status.lock().unwrap().snapshot = snap;
    if let Err(e) = api::start_long(&state, "startup", serde_json::Value::Null, None) {
        tracing::warn!("startup: {e:?}");
    }

    let static_dir = static_dir.unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../frontend/dist"));
    let app = axum::Router::new().nest("/api", api::router(state));
    let app = if static_dir.join("index.html").exists() {
        app.fallback_service(ServeDir::new(&static_dir).fallback(ServeFile::new(static_dir.join("index.html"))))
    } else {
        tracing::warn!("frontend not built ({}); use the Vite dev server (npm run dev in frontend/)", static_dir.display());
        app
    };

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let url = format!("http://{addr}");
    tracing::info!("project {}  —  GUI at {url}", dir.display());
    if open {
        let _ = std::process::Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" }).arg(&url).spawn();
    }
    axum::serve(listener, app).with_graceful_shutdown(async { let _ = tokio::signal::ctrl_c().await; }).await?;
    Ok(())
}
