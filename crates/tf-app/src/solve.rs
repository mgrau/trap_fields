//! BEM solve on the current surface mesh: electrode grouping from the labels, basis
//! solutions, solid-harmonic expansions, and output files.

use crate::project::{Label, Project, Role};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;
use tf_bem::trap::{depth_estimate, energy_direct, inside_surface, Fields, Ion, E_CHARGE};
use crate::app::Event;
use tf_bem::{expand, moment_table, Conductor, Expansion, Method, Problem, Progress, Solution, SolveOptions, Stage, Timings, V3};
use tf_geom::{Model, SurfaceMesh};

/// Largest dense system we attempt (8·n² bytes ≈ 29 GB at the limit).
pub const MAX_PANELS: usize = 60_000;
/// Largest compressed system we attempt.
pub const MAX_PANELS_COMPRESSED: usize = 400_000;
/// "auto" uses the compressed solver from this many panels on.
pub const AUTO_COMPRESSED_FROM: usize = 5_000;

/// Linear solver choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SolverChoice {
    /// Dense below AUTO_COMPRESSED_FROM panels, compressed above.
    #[default]
    Auto,
    /// Full matrix + LU: exact, O(N²) memory, O(N³) time.
    Dense,
    /// H-matrix (ACA) + GMRES: ~1e-7 relative agreement with dense, far less memory and time.
    Compressed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolveParams {
    /// Highest solid-harmonic order of the expansions.
    #[serde(default = "default_lmax")]
    pub lmax: usize,
    /// Expansion center (trap frame, m); default: the mesh refinement center.
    #[serde(default)]
    pub center: Option<V3>,
    /// Length scale of the expansion (m); default: distance from the center to the nearest electrode.
    #[serde(default)]
    pub r0: Option<f64>,
    #[serde(default)]
    pub method: SolverChoice,
    /// Accuracy settings of the compressed solver.
    #[serde(default)]
    pub settings: SolverSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolverSettings {
    /// Relative accuracy of each ACA-compressed block.
    pub aca_eps: f64,
    /// GMRES relative residual target.
    pub gmres_tol: f64,
    /// Admissibility: blocks with min(diam) <= eta * dist are compressed.
    pub eta: f64,
    /// Cluster-tree leaf size (panels).
    pub leaf_size: usize,
}

impl Default for SolverSettings {
    fn default() -> Self {
        SolverSettings { aca_eps: 1e-6, gmres_tol: 1e-9, eta: 2.0, leaf_size: 64 }
    }
}

impl SolverSettings {
    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.aca_eps > 0.0 && self.aca_eps < 0.1, "ACA tolerance must be in (0, 0.1)");
        anyhow::ensure!(self.gmres_tol > 0.0 && self.gmres_tol < 0.1, "GMRES tolerance must be in (0, 0.1)");
        anyhow::ensure!(self.eta > 0.0 && self.eta <= 10.0, "eta must be in (0, 10]");
        anyhow::ensure!((4..=1024).contains(&self.leaf_size), "leaf size must be 4..1024");
        Ok(())
    }
}

fn default_lmax() -> usize {
    10
}

impl Default for SolveParams {
    fn default() -> Self {
        SolveParams { lmax: default_lmax(), center: None, r0: None, method: SolverChoice::Auto, settings: SolverSettings::default() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Electrode {
    pub name: String,
    pub role: Role,
    pub floating: bool,
    pub bodies: Vec<String>,
}

/// Electrode name and floating flag per body: what the solution depends on.
pub type Signature = Vec<(String, bool)>;

pub fn electrodes(model: &Model, labels: &BTreeMap<String, Label>) -> (Vec<Electrode>, Vec<usize>, Signature) {
    let mut list: Vec<Electrode> = Vec::new();
    let mut body_electrode = Vec::with_capacity(model.bodies.len());
    let mut sig = Vec::with_capacity(model.bodies.len());
    for b in &model.bodies {
        let label = labels.get(&b.name);
        let name = label.map(|l| l.electrode.clone()).unwrap_or_else(|| b.name.clone());
        let role = label.map(|l| l.role).unwrap_or_default();
        let k = match list.iter().position(|e| e.name == name) {
            Some(k) => k,
            None => {
                list.push(Electrode { name: name.clone(), role, floating: false, bodies: vec![] });
                list.len() - 1
            }
        };
        let e = &mut list[k];
        e.bodies.push(b.name.clone());
        e.floating |= role == Role::Floating;
        if e.role == Role::Unassigned {
            e.role = role;
        }
        body_electrode.push(k);
        sig.push((name, role == Role::Floating));
    }
    (list, body_electrode, sig)
}

pub struct SolveResult {
    /// Identifies this solution across restarts (saved with it); results computed from
    /// it (sweeps, chains, slices) are saved under this id.
    pub id: u64,
    pub mesh_version: u64,
    pub signature: Signature,
    pub electrodes: Vec<Electrode>,
    pub problem: Problem,
    pub solution: Solution,
    pub expansions: Vec<Expansion>,
    pub params: SolveParams,
    pub center: V3,
    pub r0: f64,
    /// Distance from the center to the nearest panel: the expansions converge inside it.
    pub r_min: f64,
    pub seconds: f64,
    pub timings: Timings,
    pub expand_seconds: f64,
}

pub fn run(
    mesh: &SurfaceMesh,
    mesh_version: u64,
    model: &Model,
    labels: &BTreeMap<String, Label>,
    params: SolveParams,
    sink: &(dyn Fn(Event) + Sync),
    stop: &(dyn Fn() -> bool + Sync),
) -> anyhow::Result<SolveResult> {
    let t0 = Instant::now();
    let n = mesh.tri_body.len();
    anyhow::ensure!(n > 0, "the mesh is empty");
    anyhow::ensure!(
        mesh.stats.defective_faces.is_empty(),
        "the mesh has folded or duplicate triangles on CAD faces {:?}; regenerate it with smaller sizes",
        mesh.stats.defective_faces
    );
    let method = match params.method {
        SolverChoice::Dense => Method::Dense,
        SolverChoice::Compressed => Method::Compressed,
        SolverChoice::Auto if n >= AUTO_COMPRESSED_FROM => Method::Compressed,
        SolverChoice::Auto => Method::Dense,
    };
    if method == Method::Dense {
        anyhow::ensure!(
            n <= MAX_PANELS,
            "{n} panels is too many for the dense solver (limit {MAX_PANELS}, ~{:.0} GB); use the compressed solver or coarsen the mesh",
            8.0 * (n as f64).powi(2) / 1e9
        );
    } else {
        anyhow::ensure!(n <= MAX_PANELS_COMPRESSED, "{n} panels is too many (limit {MAX_PANELS_COMPRESSED}); coarsen the mesh");
    }
    anyhow::ensure!((1..=40).contains(&params.lmax), "lmax must be between 1 and 40");
    params.settings.validate()?;
    let (electrodes, body_electrode, signature) = electrodes(model, labels);
    let tri_cond: Vec<usize> = mesh.tri_body.iter().map(|&b| body_electrode[b as usize]).collect();
    // electrodes whose bodies all failed to mesh have no panels: drop them
    let mut used = vec![false; electrodes.len()];
    tri_cond.iter().for_each(|&k| used[k] = true);
    if let Some(k) = used.iter().position(|u| !u) {
        anyhow::bail!("electrode '{}' has no mesh panels", electrodes[k].name);
    }
    let conductors = electrodes.iter().map(|e| Conductor { name: e.name.clone(), floating: e.floating }).collect();
    let problem = Problem::from_mesh(&mesh.nodes, &mesh.triangles, tri_cond, conductors)?;

    let mut opts = SolveOptions { method, ..Default::default() };
    let st = &params.settings;
    opts.h.aca_eps = st.aca_eps;
    opts.h.eta = st.eta;
    opts.h.leaf_size = st.leaf_size;
    opts.gmres.tol = st.gmres_tol;
    let solution = tf_bem::solve_with_stop(&problem, &opts, &|p| match p {
        Progress::Stage(s) => sink(Event::Stage {
            stage: match s {
                Stage::Assembling if method == Method::Compressed => "building H-matrix",
                Stage::Assembling => "assembling",
                Stage::Factoring if method == Method::Compressed => "preconditioner",
                Stage::Factoring => "factoring",
                Stage::Solving if method == Method::Compressed => "GMRES",
                Stage::Solving => "solving",
            }
            .into(),
        }),
        Progress::Iteration { iteration, residuals } => sink(Event::Gmres { iteration, residuals: residuals.to_vec() }),
    }, stop)?;

    sink(Event::Stage { stage: "expanding".into() });
    let t1 = Instant::now();
    let center = params.center.unwrap_or(mesh.params.center);
    let r_min = problem.distance_to_surface(center);
    anyhow::ensure!(r_min > 0.0, "the expansion center lies on an electrode surface");
    let r0 = params.r0.unwrap_or(r_min);
    anyhow::ensure!(r0 > 0.0 && r0.is_finite(), "r0 must be positive");
    let table = moment_table(&problem.panels, center, r0, params.lmax);
    let expansions = solution.u.iter().map(|u| expand(&table, u, center, r0, params.lmax)).collect();
    let expand_seconds = t1.elapsed().as_secs_f64();

    let timings = solution.timings.clone();
    Ok(SolveResult {
        id: new_id(),
        mesh_version,
        signature,
        electrodes,
        problem,
        solution,
        expansions,
        params,
        center,
        r0,
        r_min,
        seconds: t0.elapsed().as_secs_f64(),
        timings,
        expand_seconds,
    })
}

impl SolveResult {
    pub fn summary(&self, version: u64) -> serde_json::Value {
        let s = &self.solution;
        let basis: Vec<serde_json::Value> = s
            .basis
            .iter()
            .enumerate()
            .map(|(b, &k)| {
                let e = &self.expansions[b];
                json!({
                    "electrode": self.electrodes[k].name,
                    "index": k,
                    "charge": s.charge[b],
                    "potential": s.potential[b],
                    "coeffs": e.coeffs,
                    "local": e.local(),
                    "sigma_range": range(&s.u[b]).map(|(lo, hi)| [lo * tf_bem::EPS0, hi * tf_bem::EPS0]),
                })
            })
            .collect();
        json!({
            "id": self.id.to_string(),
            "version": version,
            "mesh_version": self.mesh_version,
            "n_panels": self.problem.panels.len(),
            "n_unknowns": s.n_unknowns,
            "matrix_bytes": s.stats.matrix_bytes,
            "timings": self.timings,
            "solver": self.solution.stats,
            "expand_seconds": self.expand_seconds,
            "seconds": self.seconds,
            "params": self.params,
            "center": self.center,
            "r0": self.r0,
            "r_min": self.r_min,
            "lmax": self.params.lmax,
            "electrodes": self.electrodes,
            "basis": basis,
        })
    }

    /// Weighted sum of basis densities u = σ/ε0 for the given electrode voltages.
    pub fn combine(&self, voltages: &BTreeMap<String, f64>) -> anyhow::Result<(Vec<f64>, Option<Expansion>)> {
        for name in voltages.keys() {
            anyhow::ensure!(self.electrodes.iter().any(|e| &e.name == name), "unknown electrode '{name}'");
        }
        let n = self.problem.panels.len();
        let mut u = vec![0.0; n];
        let mut items = Vec::new();
        for (b, &k) in self.solution.basis.iter().enumerate() {
            let v = voltages.get(&self.electrodes[k].name).copied().unwrap_or(0.0);
            if v != 0.0 {
                u.iter_mut().zip(&self.solution.u[b]).for_each(|(a, x)| *a += v * x);
            }
            items.push((v, &self.expansions[b]));
        }
        Ok((u, Expansion::combine(items)))
    }
}

fn range(v: &[f64]) -> Option<(f64, f64)> {
    v.iter().fold(None, |acc, &x| match acc {
        None => Some((x, x)),
        Some((lo, hi)) => Some((lo.min(x), hi.max(x))),
    })
}

/// TFS1 | u32 n_basis | u32 n_tri | u32 pad | f64 sigma[n_basis][n_tri] (C/m²)
pub fn encode_sigma(r: &SolveResult) -> Vec<u8> {
    let (nb, nt) = (r.solution.u.len(), r.problem.panels.len());
    let mut buf = Vec::with_capacity(16 + 8 * nb * nt);
    buf.extend_from_slice(b"TFS1");
    // the last header word: low bits of the solution id, to match solution.json on load
    for x in [nb as u32, nt as u32, r.id as u32] {
        buf.extend_from_slice(&x.to_le_bytes());
    }
    for u in &r.solution.u {
        for x in u {
            buf.extend_from_slice(&(x * tf_bem::EPS0).to_le_bytes());
        }
    }
    buf
}

/// Identifies the mesh a saved solution belongs to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct MeshKey {
    step_file: Option<String>,
    transforms: Vec<tf_geom::TransformOp>,
    mesh_params: tf_geom::MeshParams,
    n_triangles: usize,
}

/// Write a file completely or not at all.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

/// A practically unique id (time-based) for a new solution.
fn new_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    (t ^ N.fetch_add(1, Ordering::Relaxed).wrapping_mul(0x9E37_79B9_7F4A_7C15)) | 1
}

fn mesh_key(project: &Project, mesh: &SurfaceMesh) -> MeshKey {
    MeshKey {
        step_file: project.step_file.clone(),
        transforms: project.transforms.clone(),
        mesh_params: mesh.params.clone(),
        n_triangles: mesh.tri_body.len(),
    }
}

pub fn save(dir: &Path, r: &SolveResult, version: u64, project: &Project, mesh: &SurfaceMesh) -> anyhow::Result<()> {
    let out = dir.join("solve");
    std::fs::create_dir_all(&out)?;
    let mut v = r.summary(version);
    v["id"] = json!(r.id); // a number here (the GUI gets a string: u64 exceeds JS integers)
    v["mesh_key"] = json!(mesh_key(project, mesh));
    v["format"] = json!(
        "Basis solutions: 1 V on `electrode`, 0 V on the other non-floating electrodes. \
         phi(r) = sum_lm coeffs[l*l + l + m] * Y_lm((r - center)/r0), real Racah-normalized \
         regular solid harmonics (see README). charge[k]: C on electrode k (capacitance matrix row); \
         sigma.bin: TFS1 | u32 n_basis | u32 n_tri | u32 0 | f64 sigma[n_basis][n_tri] in C/m^2, \
         triangles in the order of mesh/surface_mesh.bin."
    );
    // each file written whole (temporary + rename); sigma first, solution.json last
    write_atomic(&out.join("sigma.bin"), &encode_sigma(r))?;
    write_atomic(&out.join("solution.json"), serde_json::to_string_pretty(&v)?.as_bytes())?;
    Ok(())
}

/// Drive and ion for the pseudopotential analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrapParams {
    pub ion: Ion,
    /// RF frequency Ω/2π (Hz).
    pub rf_freq: f64,
    /// RF amplitude (V, zero to peak) per electrode.
    #[serde(default)]
    pub rf: BTreeMap<String, f64>,
    /// Static voltage per electrode (V).
    #[serde(default)]
    pub dc: BTreeMap<String, f64>,
    /// Also estimate the trap depth from the surface charges (slower).
    #[serde(default)]
    pub depth: bool,
}

pub struct Drive {
    pub fields: Fields,
    pub u_rf: Vec<f64>,
    pub u_dc: Vec<f64>,
}

impl TrapParams {
    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.ion.mass_amu > 0.0, "ion mass must be positive");
        anyhow::ensure!(self.ion.charge_e != 0.0, "ion charge must be non-zero");
        anyhow::ensure!(self.rf_freq > 0.0, "RF frequency must be positive");
        Ok(())
    }
}

impl SolveResult {
    pub fn drive(&self, p: &TrapParams) -> anyhow::Result<Drive> {
        p.validate()?;
        let (u_rf, rf) = self.combine(&p.rf)?;
        let (u_dc, dc) = self.combine(&p.dc)?;
        let (rf, dc) = (rf.ok_or_else(|| anyhow::anyhow!("no basis solutions"))?, dc.ok_or_else(|| anyhow::anyhow!("no basis solutions"))?);
        let fields = Fields { rf, dc, omega: 2.0 * std::f64::consts::PI * p.rf_freq, ion: p.ion, r_valid: self.r_min };
        Ok(Drive { fields, u_rf, u_dc })
    }

    pub fn analyze(&self, p: &TrapParams) -> anyhow::Result<serde_json::Value> {
        let t0 = Instant::now();
        let d = self.drive(p)?;
        let a = d.fields.analyze();
        let depth = (p.depth && a.confined).then(|| {
            depth_estimate(&self.problem.panels, &d.u_rf, &d.u_dc, p.ion, d.fields.omega, a.equilibrium, a.energy_eq, 10.0 * self.r_min)
        });
        Ok(json!({
            "analysis": a,
            "depth": depth,
            "depth_ev": depth.as_ref().map(|x| x.depth / E_CHARGE),
            "energy_eq_ev": a.energy_eq / E_CHARGE,
            "r_valid": self.r_min,
            "center": self.center,
            "seconds": t0.elapsed().as_secs_f64(),
        }))
    }

    /// U (eV) on an n×n grid in the plane through `center` normal to `axis`
    /// (half-width `half`); null inside electrodes.
    pub fn slice(&self, p: &TrapParams, axis: usize, center: V3, half: f64, n: usize) -> anyhow::Result<serde_json::Value> {
        anyhow::ensure!(axis < 3 && (2..=401).contains(&n) && half > 0.0, "invalid slice");
        let d = self.drive(p)?;
        let (iu, iv) = [(1, 2), (2, 0), (0, 1)][axis];
        let mut pts = Vec::with_capacity(n * n);
        for j in 0..n {
            for i in 0..n {
                let mut x = center;
                x[iu] += half * (2.0 * i as f64 / (n - 1) as f64 - 1.0);
                x[iv] += half * (2.0 * j as f64 / (n - 1) as f64 - 1.0);
                pts.push(x);
            }
        }
        let panels = &self.problem.panels;
        let inside: Vec<bool> = {
            use rayon::prelude::*;
            pts.par_iter().map(|&x| inside_surface(panels, x)).collect()
        };
        let u = energy_direct(panels, &d.u_rf, &d.u_dc, p.ion, d.fields.omega, &pts);
        let values: Vec<Option<f32>> = u.iter().zip(&inside).map(|(&e, &ins)| (!ins && e.is_finite()).then_some((e / E_CHARGE) as f32)).collect();
        Ok(json!({ "axis": axis, "u_axis": iu, "v_axis": iv, "center": center, "half": half, "n": n, "values": values }))
    }
}

#[derive(Deserialize)]
struct SavedBasis {
    index: usize,
    charge: Vec<f64>,
    potential: Vec<f64>,
}

#[derive(Deserialize)]
struct SavedSolution {
    #[serde(default)]
    id: u64,
    mesh_key: MeshKey,
    params: SolveParams,
    center: V3,
    r0: f64,
    electrodes: Vec<Electrode>,
    basis: Vec<SavedBasis>,
    n_unknowns: usize,
    timings: Timings,
    seconds: f64,
    #[serde(default)]
    solver: Option<tf_bem::SolverStats>,
}

/// Rebuild a saved solution (solve/solution.json + sigma.bin) for `mesh`, if it was
/// computed on this mesh. Expansions are recomputed from the charges.
pub fn load(dir: &Path, mesh: &SurfaceMesh, mesh_version: u64, model: &Model, project: &Project) -> anyhow::Result<Option<SolveResult>> {
    let d = dir.join("solve");
    let (Ok(js), Ok(bin)) = (std::fs::read_to_string(d.join("solution.json")), std::fs::read(d.join("sigma.bin"))) else {
        return Ok(None);
    };
    let saved: SavedSolution = match serde_json::from_str(&js) {
        Ok(s) => s,
        Err(_) => return Ok(None), // older format
    };
    if saved.mesh_key != mesh_key(project, mesh) {
        return Ok(None);
    }
    anyhow::ensure!(bin.len() >= 16 && &bin[0..4] == b"TFS1", "bad sigma file");
    let nb = u32::from_le_bytes(bin[4..8].try_into()?) as usize;
    let nt = u32::from_le_bytes(bin[8..12].try_into()?) as usize;
    anyhow::ensure!(
        nt == mesh.tri_body.len() && nb == saved.basis.len() && bin.len() as u64 == 16 + 8 * nb as u64 * nt as u64,
        "sigma file does not match the mesh"
    );
    let tag = u32::from_le_bytes(bin[12..16].try_into()?);
    anyhow::ensure!(saved.id == 0 || tag == 0 || tag == saved.id as u32, "sigma file belongs to another solution");
    let sigma: Vec<f64> = bin[16..].chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap())).collect();

    let electrodes_n = saved.electrodes.len();
    anyhow::ensure!(saved.basis.iter().all(|b| b.index < electrodes_n && b.charge.len() == electrodes_n), "inconsistent solution file");
    // body -> electrode from the saved grouping
    let electrodes = saved.electrodes;
    let mut body_electrode = Vec::with_capacity(model.bodies.len());
    let mut signature = Vec::with_capacity(model.bodies.len());
    for b in &model.bodies {
        let k = electrodes.iter().position(|e| e.bodies.contains(&b.name)).ok_or_else(|| anyhow::anyhow!("body '{}' missing from the saved solution", b.name))?;
        body_electrode.push(k);
        signature.push((electrodes[k].name.clone(), electrodes[k].floating));
    }
    let tri_cond: Vec<usize> = mesh.tri_body.iter().map(|&b| body_electrode[b as usize]).collect();
    let conductors = electrodes.iter().map(|e| Conductor { name: e.name.clone(), floating: e.floating }).collect();
    let problem = Problem::from_mesh(&mesh.nodes, &mesh.triangles, tri_cond, conductors)?;
    let u: Vec<Vec<f64>> = sigma.chunks_exact(nt).map(|c| c.iter().map(|x| x / tf_bem::EPS0).collect()).collect();

    let t1 = Instant::now();
    let r_min = problem.distance_to_surface(saved.center);
    let table = moment_table(&problem.panels, saved.center, saved.r0, saved.params.lmax);
    let expansions = u.iter().map(|u| expand(&table, u, saved.center, saved.r0, saved.params.lmax)).collect();
    let solution = Solution {
        basis: saved.basis.iter().map(|b| b.index).collect(),
        u,
        charge: saved.basis.iter().map(|b| b.charge.clone()).collect(),
        potential: saved.basis.iter().map(|b| b.potential.clone()).collect(),
        n_unknowns: saved.n_unknowns,
        timings: saved.timings.clone(),
        stats: saved.solver.clone().unwrap_or_default(),
    };
    Ok(Some(SolveResult {
        // solutions saved before ids existed: a stable id from the file contents
        id: if saved.id != 0 { saved.id } else { js.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3)) | 1 },
        mesh_version,
        signature,
        electrodes,
        problem,
        solution,
        expansions,
        params: saved.params,
        center: saved.center,
        r0: saved.r0,
        r_min,
        seconds: saved.seconds,
        timings: saved.timings,
        expand_seconds: t1.elapsed().as_secs_f64(),
    }))
}

/// What a frequency sweep varies.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SweepVar {
    /// RF amplitude (V) of every RF electrode (signs kept, all set to ±value).
    RfAmplitude,
    /// RF frequency Ω/2π (Hz).
    RfFreq,
    /// Static voltage (V) of one electrode.
    Dc { electrode: String },
}

impl SolveResult {
    /// Single-ion analysis over a range of one drive parameter.
    pub fn sweep(&self, p: &TrapParams, var: &SweepVar, values: &[f64]) -> anyhow::Result<serde_json::Value> {
        anyhow::ensure!(!values.is_empty() && values.len() <= 2000, "1 to 2000 sweep points");
        if let SweepVar::Dc { electrode } = var {
            anyhow::ensure!(self.electrodes.iter().any(|e| &e.name == electrode && !e.floating), "unknown electrode '{electrode}'");
        }
        let t0 = Instant::now();
        let rows: Vec<serde_json::Value> = values
            .iter()
            .map(|&v| {
                let mut q = p.clone();
                match var {
                    SweepVar::RfAmplitude => q.rf.values_mut().for_each(|a| {
                        if *a != 0.0 {
                            *a = a.signum() * v
                        }
                    }),
                    SweepVar::RfFreq => q.rf_freq = v,
                    SweepVar::Dc { electrode } => {
                        q.dc.insert(electrode.clone(), v);
                    }
                }
                match self.drive(&q) {
                    Ok(d) => {
                        let a = d.fields.analyze();
                        json!({
                            "value": v,
                            "confined": a.confined,
                            "stable": a.stable,
                            "freq_pseudo": a.modes.iter().map(|m| m.freq_pseudo).collect::<Vec<_>>(),
                            "freq_floquet": a.modes.iter().map(|m| m.freq_floquet).collect::<Vec<_>>(),
                            "axes": a.modes.iter().map(|m| m.axis).collect::<Vec<_>>(),
                            "q": a.mathieu.iter().map(|m| m.q).collect::<Vec<_>>(),
                            "a": a.mathieu.iter().map(|m| m.a).collect::<Vec<_>>(),
                            "equilibrium": a.equilibrium,
                        })
                    }
                    Err(e) => json!({ "value": v, "error": format!("{e:#}") }),
                }
            })
            .collect();
        Ok(json!({ "rows": rows, "seconds": t0.elapsed().as_secs_f64() }))
    }

    /// Equilibrium positions and normal modes of `n` ions.
    pub fn chain(&self, p: &TrapParams, n: usize) -> anyhow::Result<serde_json::Value> {
        let d = self.drive(p)?;
        let t0 = Instant::now();
        let c = tf_bem::chain::solve_chain(&d.fields, n).map_err(|e| anyhow::anyhow!(e))?;
        let mut v = serde_json::to_value(&c)?;
        v["seconds"] = json!(t0.elapsed().as_secs_f64());
        Ok(v)
    }
}
