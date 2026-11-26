//! Headless use: the GUI's operations from the command line, for scripts and batch runs.
//! Every subcommand opens the project directory as the GUI does (re-loading its STEP
//! file and restoring the saved mesh and solution), runs, prints JSON on stdout and
//! progress on stderr. What it computes is saved in the project as from the GUI.

use anyhow::{bail, Context};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;
use tf_app::{App, Event, Reply};

/// Fineness presets: multiples of the suggested mesh sizes (as in the GUI's Mesh tab).
pub const FINENESS: [(&str, f64); 4] = [("coarse", 1.4), ("normal", 1.0), ("fine", 0.7), ("very-fine", 0.5)];

/// Stages of long operations on stderr, each once, with the time since the start.
fn progress_sink(quiet: bool) -> impl Fn(Event) + Sync {
    let t0 = Instant::now();
    let last = Mutex::new(String::new());
    move |e: Event| {
        if quiet {
            return;
        }
        if let Event::Stage { stage } = e {
            let mut l = last.lock().unwrap();
            if *l != stage {
                eprintln!("[{:7.1} s] {stage}", t0.elapsed().as_secs_f64());
                *l = stage;
            }
        }
    }
}

pub struct Session {
    pub app: App,
    quiet: bool,
}

impl Session {
    /// Open `dir` and restore its model, mesh and solution.
    pub fn open(dir: &Path, quiet: bool) -> anyhow::Result<Session> {
        let dir = dir.canonicalize().with_context(|| format!("project dir {}", dir.display()))?;
        let mut s = Session { app: App::new(&dir)?, quiet };
        s.run("startup", Value::Null)?;
        let st = s.state()?;
        if let JobState::Failed(e) = job(&st) {
            bail!("loading the project failed: {e}");
        }
        Ok(s)
    }

    /// Run one app command; its JSON reply.
    pub fn run(&mut self, cmd: &str, args: Value) -> anyhow::Result<Value> {
        let sink = progress_sink(self.quiet);
        match self.app.command(cmd, args, None, &sink) {
            Ok(Reply::Json(v)) => Ok(v),
            Ok(Reply::Binary(b)) => Ok(json!({ "binary_bytes": b.len() })),
            Err(e) => bail!("{cmd}: {}", e.message),
        }
    }

    pub fn state(&mut self) -> anyhow::Result<Value> {
        self.run("state", Value::Null)
    }
}

enum JobState {
    Failed(String),
    Other,
}

fn job(st: &Value) -> JobState {
    match st["job"]["state"].as_str() {
        Some("failed") => JobState::Failed(st["job"]["error"].as_str().unwrap_or("unknown error").into()),
        _ => JobState::Other,
    }
}

fn print(v: &Value) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

/// Load (if needed), mesh (if needed) and solve; print a summary.
pub fn solve(dir: &Path, step: Option<&str>, remesh: bool, fineness: Option<&str>, quiet: bool) -> anyhow::Result<()> {
    let mut s = Session::open(dir, quiet)?;
    if let Some(f) = step {
        s.run("open", json!({ "file": f }))?;
    }
    let st = s.state()?;
    if st["model"].is_null() {
        bail!("no STEP file loaded: pass --step FILE (a .step file in the project directory)");
    }
    let unassigned: Vec<&str> = st["bodies"].as_array().into_iter().flatten().filter(|b| b["label"]["role"].as_str().unwrap_or("unassigned") == "unassigned").filter_map(|b| b["name"].as_str()).collect();
    if !unassigned.is_empty() && !quiet {
        eprintln!("note: bodies without a role (solved as electrodes of their own): {}", unassigned.join(", "));
    }
    let preset = fineness.map(|f| FINENESS.iter().find(|(k, _)| *k == f).map(|x| x.1).ok_or_else(|| anyhow::anyhow!("fineness must be one of coarse, normal, fine, very-fine"))).transpose()?;
    if st["mesh"].is_null() || remesh || preset.is_some() {
        // the project's sizes, else (or with --fineness) the suggested ones scaled
        let params = match (preset, st["project"]["mesh_params"].is_null()) {
            (None, false) => st["project"]["mesh_params"].clone(),
            (scale, _) => {
                let mut p = st["mesh_defaults"].clone();
                if p.is_null() {
                    bail!("no suggested mesh sizes: is the trap center (origin) inside the model?");
                }
                let k = scale.unwrap_or(0.7);
                for key in ["h_min", "h_max"] {
                    p[key] = json!(p[key].as_f64().unwrap_or(0.0) * k);
                }
                p
            }
        };
        s.run("mesh", params)?;
    }
    let solve_params = s.state()?["project"]["solve_params"].clone();
    let solve_params = if solve_params.is_null() { json!({}) } else { solve_params };
    s.run("solve", solve_params)?;
    let sol = s.run("solution", Value::Null)?;
    let basis: Vec<Value> = sol["basis"].as_array().into_iter().flatten().map(|b| json!({ "electrode": b["electrode"], "local": b["local"], "charge": b["charge"] })).collect();
    print(&json!({
        "n_panels": sol["n_panels"],
        "seconds": sol["seconds"],
        "solver": sol["solver"]["method"],
        "center": sol["center"],
        "r_min": sol["r_min"],
        "electrodes": sol["electrodes"],
        "moves": sol["moves"],
        "basis": basis,
        "notes": s.state()?["notes"],
    }))
}

fn need_solution(s: &mut Session) -> anyhow::Result<Value> {
    let st = s.state()?;
    if st["solve"].is_null() {
        bail!("no saved solution for this project: run `trapfields solve` first");
    }
    if st["solve"]["stale"].as_bool() == Some(true) && !s.quiet {
        eprintln!("note: the electrodes or their displacements changed since the solve; results are for the solved geometry");
    }
    let p = st["project"]["trap_params"].clone();
    if p.is_null() {
        bail!("no drive in trapfields.json (trap_params): set the RF and DC voltages on the Trap tab, or with `trapfields command . trap '{{...}}'`");
    }
    Ok(p)
}

/// Trap analysis of the project's drive.
pub fn analyze(dir: &Path, depth: bool, quiet: bool) -> anyhow::Result<()> {
    let mut s = Session::open(dir, quiet)?;
    let mut p = need_solution(&mut s)?;
    p["depth"] = json!(depth);
    print(&s.run("trap", p)?)
}

pub struct ToleranceOpts<'a> {
    pub electrodes: Option<&'a str>,
    pub translate_um: f64,
    pub rotate_mrad: f64,
    pub forward: bool,
    pub sigma_um: Option<f64>,
    pub sigma_mrad: Option<f64>,
}

/// Electrode-displacement sensitivities of the project's drive (and a tolerance budget).
pub fn tolerance(dir: &Path, o: &ToleranceOpts, quiet: bool) -> anyhow::Result<()> {
    let mut s = Session::open(dir, quiet)?;
    let p = need_solution(&mut s)?;
    let electrodes: Vec<String> = o.electrodes.map(|e| e.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default();
    s.run("tolerance", json!({ "electrodes": electrodes, "translate_step": o.translate_um * 1e-6, "rotate_step": o.rotate_mrad * 1e-3, "central": !o.forward }))?;
    let mut sigma = Value::Null;
    if o.sigma_um.is_some() || o.sigma_mrad.is_some() {
        sigma = json!({ "translate": o.sigma_um.unwrap_or(0.0) * 1e-6, "rotate": o.sigma_mrad.unwrap_or(0.0) * 1e-3 });
    }
    print(&s.run("tolerance_eval", json!({ "params": p, "sigma": sigma }))?)
}

/// Any app command, as the GUI sends it.
pub fn command(dir: &Path, name: &str, args: Option<&str>, quiet: bool) -> anyhow::Result<()> {
    let mut s = Session::open(dir, quiet)?;
    let args: Value = match args {
        Some(a) => serde_json::from_str(a).context("arguments must be JSON")?,
        None => Value::Null,
    };
    print(&s.run(name, args)?)
}
