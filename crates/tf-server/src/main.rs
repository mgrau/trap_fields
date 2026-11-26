//! `trapfields` — RF Paul trap field simulator.

mod api;
mod cli;
mod worker;

use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use axum::http::{HeaderName, HeaderValue};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

#[derive(Parser)]
#[command(name = "trapfields", version, about = "RF Paul trap field simulator")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Load the project, mesh it if it has no mesh yet, and solve (headless).
    Solve {
        #[arg(default_value = ".")]
        project: PathBuf,
        /// Open this STEP file of the project directory first.
        #[arg(long)]
        step: Option<String>,
        /// Mesh again even if a saved mesh matches.
        #[arg(long)]
        remesh: bool,
        /// Mesh with the suggested sizes at this fineness: coarse, normal, fine, very-fine.
        #[arg(long)]
        fineness: Option<String>,
        /// No progress on stderr.
        #[arg(long, short)]
        quiet: bool,
    },
    /// Trap analysis (frequencies, Mathieu parameters, …) of the project's drive, as JSON.
    Analyze {
        #[arg(default_value = ".")]
        project: PathBuf,
        /// Also the trap depth (slower).
        #[arg(long)]
        depth: bool,
        #[arg(long, short)]
        quiet: bool,
    },
    /// Sensitivity of the trap to displacing each electrode, as JSON.
    Tolerance {
        #[arg(default_value = ".")]
        project: PathBuf,
        /// Electrodes to displace, comma-separated (default: all).
        #[arg(long)]
        electrodes: Option<String>,
        /// Translation step (µm).
        #[arg(long, default_value_t = 5.0)]
        translate_um: f64,
        /// Rotation step (mrad).
        #[arg(long, default_value_t = 1.0)]
        rotate_mrad: f64,
        /// One step forward instead of one each way (half the solves, no second derivatives).
        #[arg(long)]
        forward: bool,
        /// Tolerance budget: standard deviation of every translation component (µm).
        #[arg(long)]
        sigma_um: Option<f64>,
        /// Tolerance budget: standard deviation of every rotation component (mrad).
        #[arg(long)]
        sigma_mrad: Option<f64>,
        #[arg(long, short)]
        quiet: bool,
    },
    /// Create an example project (a generated trap, labeled, with a drive) in a new directory.
    Demo {
        dir: PathBuf,
        /// linear (four rods, needle endcaps; the default), or one of the examples/ traps:
        /// paul (March's stretched quadrupole trap), surface (five-wire surface-electrode
        /// trap), segmented (segmented four-rod trap); paul-ideal (unstretched hyperbolic).
        #[arg(long, default_value = "linear")]
        design: String,
    },
    /// Run any app command (as the GUI sends it) after loading the project; prints its JSON reply.
    Command {
        project: PathBuf,
        /// Command name (e.g. state, trap, sweep, chain, moves, export_fields).
        name: String,
        /// Arguments as JSON.
        args: Option<String>,
        #[arg(long, short)]
        quiet: bool,
    },
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
        Cmd::Solve { project, step, remesh, fineness, quiet } => cli::solve(&project, step.as_deref(), remesh, fineness.as_deref(), quiet),
        Cmd::Analyze { project, depth, quiet } => cli::analyze(&project, depth, quiet),
        Cmd::Tolerance { project, electrodes, translate_um, rotate_mrad, forward, sigma_um, sigma_mrad, quiet } => {
            cli::tolerance(&project, &cli::ToleranceOpts { electrodes: electrodes.as_deref(), translate_um, rotate_mrad, forward, sigma_um, sigma_mrad }, quiet)
        }
        Cmd::Demo { dir, design } => {
            use tf_app::demo::Design;
            let d = Design::parse(&design).ok_or_else(|| anyhow::anyhow!("unknown design {design:?}: one of linear, paul, surface, segmented, paul-ideal"))?;
            tf_app::demo::create_design(&dir, d)?;
            println!("Created {} with {} ({}). Next: trapfields solve {0}, or trapfields gui {0} --open", dir.display(), d.step_file(), d.describe());
            Ok(())
        }
        Cmd::Command { project, name, args, quiet } => cli::command(&project, &name, args.as_deref(), quiet),
    }
}

async fn gui(project: PathBuf, port: u16, open: bool, static_dir: Option<PathBuf>) -> anyhow::Result<()> {
    let dir = project.canonicalize().map_err(|e| anyhow::anyhow!("project dir {}: {e}", project.display()))?;
    let state = Arc::new(api::Server {
        worker: worker::AppWorker::spawn(dir.clone())?,
        status: Default::default(),
    });
    // restarted to force a stop (see api::cancel)?
    let stopped: Option<serde_json::Value> = std::env::var(api::STOPPED_ENV).ok().and_then(|v| serde_json::from_str(&v).ok());
    std::env::remove_var(api::STOPPED_ENV);
    // initial snapshot, then re-open the project's STEP file (and restore its mesh and
    // solution) in the background
    let snap = state.worker.run(|app| app.state()).await?;
    let step_file = snap["project"]["step_file"].as_str().map(String::from);
    let mut reopen = true;
    let mut notice = None;
    if let Some(st) = &stopped {
        let skip = st["skip_reopen"].as_str();
        // the file that would not stop loading must not hang the restarted server too
        reopen = step_file.is_some() && skip != Some("*") && skip != step_file.as_deref();
        let mut n = "Stopped. The operation could not be interrupted, so the server was restarted.".to_string();
        match (&step_file, reopen) {
            (Some(f), true) => n += &format!(" Re-opening {f}."),
            (Some(f), false) => n += &format!(" {f} was not re-opened: choose it under Project to load it."),
            _ => {}
        }
        notice = Some(n);
    }
    state.status.lock().unwrap().snapshot = snap;
    if reopen {
        if let Err(e) = api::start_long(&state, "startup", serde_json::Value::Null, None) {
            tracing::warn!("startup: {e:?}");
        }
    }
    // after starting the re-open, which clears notices of earlier jobs
    state.status.lock().unwrap().notice = notice;

    let static_dir = static_dir.unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../frontend/dist"));
    let app = axum::Router::new().nest("/api", api::router(state));
    let app = if static_dir.join("index.html").exists() {
        app.fallback_service(ServeDir::new(&static_dir).fallback(ServeFile::new(static_dir.join("index.html"))))
    } else {
        tracing::warn!("frontend not built ({}); use the Vite dev server (npm run dev in frontend/)", static_dir.display());
        app
    };
    // cross-origin isolation, so the GUI can also run the in-browser engine (?engine=browser)
    let app = app
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("cross-origin-opener-policy"),
            HeaderValue::from_static("same-origin"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("cross-origin-embedder-policy"),
            HeaderValue::from_static("require-corp"),
        ));

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
