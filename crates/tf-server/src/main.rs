//! `trapfields` — RF Paul trap field simulator.

mod api;
mod project;
mod solve;
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
    let state = Arc::new(api::AppState {
        dir: dir.clone(),
        worker: worker::GmshWorker::spawn()?,
        inner: tokio::sync::Mutex::new(api::Inner { project: project::Project::load(&dir)?, ..Default::default() }),
    });

    // re-open the project's STEP file in the background
    let last = state.inner.lock().await.project.step_file.clone();
    if let Some(file) = last {
        let s = state.clone();
        tokio::spawn(async move {
            match api::open_file(&s, &file).await.map_err(|e| e.message().to_string()) {
                Ok(()) => api::restore_outputs(&s).await,
                Err(e) => tracing::warn!("could not re-open {file}: {e}"),
            }
        });
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
