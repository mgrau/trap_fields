//! How the trap responds to small rigid displacements of each electrode: a sensitivity
//! study by finite differences.
//!
//! For every chosen electrode and each of its six rigid degrees of freedom (translations
//! along x, y, z and rotations about axes parallel to x, y, z through its center) the
//! panels of that electrode are moved by a small step and the basis solutions are solved
//! again on the same, moved, mesh. Only the expansion coefficients are kept, so the
//! response of any drive (RF and DC voltages, ion, frequency) follows in milliseconds:
//! the trap analysis runs on the perturbed expansions and the differences give the
//! derivatives of the ion position, RF null, RF field at the ion (excess micromotion),
//! secular frequencies and principal axes with respect to every displacement. With
//! central differences (a step each way) the second derivatives come too.
//!
//! The perturbations compose with the electrode moves of the current solution: the study
//! linearizes about the geometry that was solved.

use crate::app::Event;
use crate::moves::Move;
use crate::solve::{solve_problem, SolveResult, TrapParams};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Instant;
use tf_bem::harmonics::{n_coeffs, panel_moments};
use tf_bem::trap::{Analysis, Fields};
use tf_bem::{moment_table, Expansion, Problem, V3};

/// Names of the six degrees of freedom of an electrode.
pub const DOFS: [&str; 6] = ["x", "y", "z", "θx", "θy", "θz"];

/// Most perturbed solves in one study.
const MAX_CASES: usize = 600;

#[derive(Debug, Clone, Deserialize)]
pub struct ToleranceArgs {
    /// Electrodes to displace (default: all).
    #[serde(default)]
    pub electrodes: Vec<String>,
    /// Translation step (m).
    #[serde(default = "default_translate")]
    pub translate_step: f64,
    /// Rotation step (rad).
    #[serde(default = "default_rotate")]
    pub rotate_step: f64,
    /// A step each way (second derivatives, error O(h²)) instead of one forward step.
    #[serde(default = "yes")]
    pub central: bool,
    /// ACA tolerance of the compressed solver for these solves (default: 1e-8, or the
    /// solution's if tighter). Differences between solves are what count here, and each
    /// solve compresses its matrix a little differently: quantities that are themselves
    /// small differences, like the RF field at the ion, need the tighter setting.
    #[serde(default)]
    pub aca_eps: Option<f64>,
}

/// Solver accuracy of the study's solves: at least ACA 1e-8 and GMRES 1e-10.
fn study_params(sol: &SolveResult, args: &ToleranceArgs) -> crate::solve::SolveParams {
    let mut p = sol.params.clone();
    p.settings.aca_eps = args.aca_eps.unwrap_or(p.settings.aca_eps.min(1e-8));
    p.settings.gmres_tol = p.settings.gmres_tol.min(1e-10);
    p
}

fn default_translate() -> f64 {
    5e-6
}
fn default_rotate() -> f64 {
    1e-3
}
fn yes() -> bool {
    true
}

/// One perturbed solve.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Case {
    /// Index into [`ToleranceData::electrodes`].
    pub electrode: usize,
    /// 0..6, see [`DOFS`].
    pub dof: usize,
    /// +1 or −1: the direction of the step.
    pub sign: f64,
    /// Expansion coefficients of every basis solution, in the solution's basis order.
    pub coeffs: Vec<Vec<f64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToleranceData {
    /// The solution the study perturbs.
    pub solution_id: u64,
    /// Expansion coefficients of the unperturbed geometry, solved with the study's own
    /// accuracy (the reference of forward and second differences).
    pub nominal: Vec<Vec<f64>>,
    pub aca_eps: f64,
    pub electrodes: Vec<String>,
    /// Point each electrode's rotations turn about (m): its center, moved with it.
    pub pivots: Vec<V3>,
    pub translate_step: f64,
    pub rotate_step: f64,
    pub central: bool,
    pub cases: Vec<Case>,
    /// Cases planned (fewer finished if the study was stopped).
    pub planned: usize,
    pub seconds: f64,
}

impl ToleranceData {
    /// What the GUI shows about the study (without the coefficients).
    pub fn summary(&self) -> Value {
        json!({
            "electrodes": self.electrodes,
            "pivots": self.pivots,
            "translate_step": self.translate_step,
            "rotate_step": self.rotate_step,
            "central": self.central,
            "aca_eps": self.aca_eps,
            "cases": self.cases.len(),
            "planned": self.planned,
            "complete": self.cases.len() == self.planned,
            "seconds": self.seconds,
        })
    }

    fn step(&self, dof: usize) -> f64 {
        if dof < 3 {
            self.translate_step
        } else {
            self.rotate_step
        }
    }
}

fn unit(i: usize, s: f64) -> V3 {
    let mut v = [0.0; 3];
    v[i] = s;
    v
}

/// The displacement of one case.
fn perturbation(dof: usize, step: f64, sign: f64, pivot: V3) -> Move {
    if dof < 3 {
        Move { translate: unit(dof, sign * step), ..Default::default() }
    } else {
        Move { rotate: unit(dof - 3, sign * step), pivot: Some(pivot), ..Default::default() }
    }
}

/// Run the perturbed solves. Returns what finished (also when stopped or failed part-way)
/// and the outcome.
pub fn run(sol: &SolveResult, args: &ToleranceArgs, sink: &(dyn Fn(Event) + Sync), stop: &(dyn Fn() -> bool + Sync)) -> (Option<ToleranceData>, anyhow::Result<()>) {
    let t0 = Instant::now();
    let checked = (|| -> anyhow::Result<Vec<usize>> {
        anyhow::ensure!(args.translate_step > 0.0 && args.translate_step <= 1e-3, "the translation step must be in (0, 1 mm]");
        anyhow::ensure!(args.rotate_step > 0.0 && args.rotate_step <= 0.1, "the rotation step must be in (0, 0.1 rad]");
        let ks: Vec<usize> = if args.electrodes.is_empty() {
            (0..sol.electrodes.len()).collect()
        } else {
            args.electrodes
                .iter()
                .map(|n| sol.electrodes.iter().position(|e| &e.name == n).ok_or_else(|| anyhow::anyhow!("unknown electrode '{n}'")))
                .collect::<anyhow::Result<_>>()?
        };
        anyhow::ensure!(!ks.is_empty(), "no electrodes to displace");
        let n = ks.len() * 6 * if args.central { 2 } else { 1 };
        anyhow::ensure!(n <= MAX_CASES, "{n} solves is too many (at most {MAX_CASES}); choose fewer electrodes");
        Ok(ks)
    })();
    let ks = match checked {
        Ok(k) => k,
        Err(e) => return (None, Err(e)),
    };
    // rotations turn about each electrode's center, where its current move put it
    let pivots: Vec<V3> = ks
        .iter()
        .map(|&k| {
            let c = sol.centers.get(k).copied().unwrap_or([0.0; 3]);
            match sol.moves.get(&sol.electrodes[k].name) {
                Some(m) => m.rigid(c).apply(c),
                None => c,
            }
        })
        .collect();
    let signs: &[f64] = if args.central { &[1.0, -1.0] } else { &[1.0] };
    // the first solve is the unperturbed geometry (sign 0)
    let plan: Vec<(usize, usize, f64)> =
        std::iter::once((0, 0, 0.0)).chain((0..ks.len()).flat_map(|i| (0..6).flat_map(move |d| signs.iter().map(move |&s| (i, d, s))))).collect();
    let params = study_params(sol, args);
    let mut data = ToleranceData {
        solution_id: sol.id,
        nominal: Vec::new(),
        aca_eps: params.settings.aca_eps,
        electrodes: ks.iter().map(|&k| sol.electrodes[k].name.clone()).collect(),
        pivots: pivots.clone(),
        translate_step: args.translate_step,
        rotate_step: args.rotate_step,
        central: args.central,
        cases: Vec::new(),
        planned: plan.len() - 1,
        seconds: 0.0,
    };

    let (center, r0, lmax) = (sol.center, sol.r0, sol.params.lmax);
    let nl = n_coeffs(lmax);
    sink(Event::Stage { stage: "moment table".into() });
    let mut table = moment_table(&sol.problem.panels, center, r0, lmax);
    let total = plan.len();
    let mut outcome = Ok(());
    for (c, &(i, dof, sign)) in plan.iter().enumerate() {
        if stop() {
            outcome = Err(anyhow::anyhow!("stopped"));
            break;
        }
        let k = ks[i];
        let step = if dof < 3 { args.translate_step } else { args.rotate_step };
        let rigid = if sign == 0.0 { tf_geom::Rigid::IDENTITY } else { perturbation(dof, step, sign, pivots[i]).rigid(pivots[i]) };
        let what = if sign == 0.0 { "unperturbed".to_string() } else { format!("{} {}{}", sol.electrodes[k].name, if sign > 0.0 { '+' } else { '−' }, DOFS[dof]) };
        let label = format!("case {}/{total} ({what}): ", c + 1);
        let case_sink = |e: Event| match e {
            Event::Stage { stage } => sink(Event::Stage { stage: format!("{label}{stage}") }),
            e => sink(e),
        };
        sink(Event::Stage { stage: format!("{label}moving") });
        let moved: Vec<usize> = if sign == 0.0 { Vec::new() } else { (0..sol.problem.panels.len()).filter(|&j| sol.problem.panel_conductor[j] == k).collect() };
        let mut problem = Problem { panels: sol.problem.panels.clone(), panel_conductor: sol.problem.panel_conductor.clone(), conductors: sol.problem.conductors.clone() };
        let mut rigids = vec![None; sol.electrodes.len()];
        if sign != 0.0 {
            rigids[k] = Some(rigid);
        }
        if let Err(e) = crate::moves::apply(&mut problem, &rigids) {
            outcome = Err(e);
            break;
        }
        let solution = match solve_problem(&problem, &params, &case_sink, stop) {
            Ok(s) => s,
            Err(e) => {
                outcome = Err(e);
                break;
            }
        };
        case_sink(Event::Stage { stage: "expanding".into() });
        // only the moved panels' moments change: swap their rows in, expand, swap back
        let mut rows = vec![0.0; moved.len() * nl];
        rows.par_chunks_mut(nl).zip(moved.par_iter()).for_each(|(row, &j)| panel_moments(&problem.panels[j], center, r0, lmax, row));
        let swap = |table: &mut [f64], rows: &mut [f64]| {
            for (r, &j) in rows.chunks_exact_mut(nl).zip(&moved) {
                table[j * nl..(j + 1) * nl].swap_with_slice(r);
            }
        };
        swap(&mut table, &mut rows);
        let coeffs = solution.u.iter().map(|u| tf_bem::expand(&table, u, center, r0, lmax).coeffs).collect();
        swap(&mut table, &mut rows);
        if sign == 0.0 {
            data.nominal = coeffs;
        } else {
            data.cases.push(Case { electrode: i, dof, sign, coeffs });
        }
        sink(Event::Progress { done: c + 1, total });
    }
    data.seconds = t0.elapsed().as_secs_f64();
    (Some(data), outcome)
}

// ------------------------------------------------------------------ evaluation

/// Scalar quantities read from an analysis, relative to the nominal one.
struct Reading {
    values: Vec<f64>,
    /// Matched principal axes (sign-aligned with the nominal ones).
    axes: [V3; 3],
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Mode `j` of the nominal analysis matched to a mode of `a` by its axis.
fn match_modes(a: &Analysis, a0: &Analysis) -> [usize; 3] {
    let mut used = [false; 3];
    let mut out = [0; 3];
    // the best-aligned pairs first
    let mut pairs: Vec<(f64, usize, usize)> = (0..3).flat_map(|j| (0..3).map(move |i| (j, i))).map(|(j, i)| (dot(a.modes[i].axis, a0.modes[j].axis).abs(), j, i)).collect();
    pairs.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut done = [false; 3];
    for (_, j, i) in pairs {
        if !done[j] && !used[i] {
            out[j] = i;
            done[j] = true;
            used[i] = true;
        }
    }
    out
}

/// The RF null; on a null line, where the line crosses the plane through the nominal ion
/// position normal to the nominal line: Newton's method on the RF gradient's two
/// components in that plane (the null search of the analysis ends anywhere along a line).
fn null_at_ion(f: &Fields, a: &Analysis, a0: &Analysis) -> V3 {
    let Some(l) = a0.rf_null_line else { return a.rf_null };
    // a basis (u, v) of the plane
    let k = (0..3).min_by(|&i, &j| l[i].abs().partial_cmp(&l[j].abs()).unwrap()).unwrap_or(0);
    let e = unit(k, 1.0);
    let cross = |a: V3, b: V3| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let u = cross(l, e);
    let nu = dot(u, u).sqrt();
    let u = u.map(|x| x / nu);
    let v = cross(l, u);
    let mv = |h: &[[f64; 3]; 3], x: V3| [dot(h[0], x), dot(h[1], x), dot(h[2], x)];
    let mut x = a0.equilibrium;
    for _ in 0..30 {
        let d = f.rf.derivatives(x);
        let (gu, gv) = (dot(u, d.grad), dot(v, d.grad));
        let (huu, huv, hvv) = (dot(u, mv(&d.hessian, u)), dot(u, mv(&d.hessian, v)), dot(v, mv(&d.hessian, v)));
        let det = huu * hvv - huv * huv;
        if det.abs() < 1e-300 {
            return a.rf_null;
        }
        let (su, sv) = (-(hvv * gu - huv * gv) / det, -(huu * gv - huv * gu) / det);
        x = [0, 1, 2].map(|i| x[i] + su * u[i] + sv * v[i]);
        if su.hypot(sv) < 1e-14 * f.rf.r0 {
            break;
        }
    }
    x
}

fn read(f: &Fields, a: &Analysis, a0: &Analysis) -> Reading {
    let m = match_modes(a, a0);
    let mut values = Vec::with_capacity(12);
    values.extend(a.equilibrium);
    values.extend(null_at_ion(f, a, a0));
    values.extend(a.rf_field_at_eq);
    let mut axes = [[0.0; 3]; 3];
    for j in 0..3 {
        let md = &a.modes[m[j]];
        values.push(md.freq_pseudo.unwrap_or(f64::NAN));
        let s = if dot(md.axis, a0.modes[j].axis) < 0.0 { -1.0 } else { 1.0 };
        axes[j] = md.axis.map(|x| s * x);
    }
    Reading { values, axes }
}

fn axis_name(v: V3) -> &'static str {
    let i = (0..3).max_by(|&a, &b| v[a].abs().partial_cmp(&v[b].abs()).unwrap()).unwrap_or(0);
    ["x", "y", "z"][i]
}

fn finite(x: f64) -> Option<f64> {
    x.is_finite().then_some(x)
}

/// Derivatives of the trap quantities for drive `p` with respect to every displacement
/// of the study.
pub fn evaluate(sol: &SolveResult, data: &ToleranceData, p: &TrapParams) -> anyhow::Result<Value> {
    anyhow::ensure!(data.solution_id == sol.id, "the tolerance study belongs to another solution: run it again");
    let t0 = Instant::now();
    // (checks the drive; its fields are the solution's, the reference is the study's own)
    let d0 = sol.drive(p)?;

    // weight of each basis solution in the RF and DC potentials
    let w: Vec<(f64, f64)> = sol
        .solution
        .basis
        .iter()
        .map(|&k| {
            let n = &sol.electrodes[k].name;
            (p.rf.get(n).copied().unwrap_or(0.0), p.dc.get(n).copied().unwrap_or(0.0))
        })
        .collect();
    let (center, r0, lmax) = (sol.center, sol.r0, sol.params.lmax);
    let fields_of = |coeffs: &[Vec<f64>]| -> Option<Fields> {
        if coeffs.len() != w.len() {
            return None;
        }
        let exps: Vec<Expansion> = coeffs.iter().map(|c| Expansion { lmax, center, r0, coeffs: c.clone() }).collect();
        let rf = Expansion::combine(w.iter().zip(&exps).map(|(w, e)| (w.0, e)))?;
        let dc = Expansion::combine(w.iter().zip(&exps).map(|(w, e)| (w.1, e)))?;
        Some(Fields { rf, dc, omega: d0.fields.omega, ion: d0.fields.ion, r_valid: sol.r_min })
    };
    let f0 = fields_of(&data.nominal).ok_or_else(|| anyhow::anyhow!("the tolerance study has no unperturbed solve"))?;
    let a0 = f0.analyze();
    anyhow::ensure!(a0.confined, "the ion is not confined with this drive: nothing to linearize about");
    let nominal = read(&f0, &a0, &a0);
    let mut notes = Vec::new();
    let readings: Vec<Option<Reading>> = data
        .cases
        .par_iter()
        .map(|c| {
            let f = fields_of(&c.coeffs)?;
            let a = f.analyze();
            a.confined.then(|| read(&f, &a, &a0))
        })
        .collect();
    for (c, r) in data.cases.iter().zip(&readings) {
        if r.is_none() {
            notes.push(format!("{} {}{}: the ion is not confined after this step; its derivatives are left out.", data.electrodes[c.electrode], if c.sign > 0.0 { '+' } else { '−' }, DOFS[c.dof]));
        }
    }

    // quantities: name, unit, group, nominal
    let mut quantities: Vec<Value> = Vec::new();
    for (i, ax) in ["x", "y", "z"].iter().enumerate() {
        quantities.push(json!({ "key": format!("eq_{ax}"), "name": format!("ion {ax}"), "unit": "m", "group": "position", "nominal": a0.equilibrium[i] }));
    }
    for (i, ax) in ["x", "y", "z"].iter().enumerate() {
        quantities.push(json!({ "key": format!("null_{ax}"), "name": format!("RF null {ax}"), "unit": "m", "group": "null", "nominal": nominal.values[3 + i] }));
    }
    for (i, ax) in ["x", "y", "z"].iter().enumerate() {
        quantities.push(json!({ "key": format!("erf_{ax}"), "name": format!("RF field at ion, {ax}"), "unit": "V/m", "group": "field", "nominal": a0.rf_field_at_eq[i] }));
    }
    for (j, m) in a0.modes.iter().enumerate() {
        quantities.push(json!({ "key": format!("f{}", j + 1), "name": format!("mode {} (≈{}) frequency", j + 1, axis_name(m.axis)), "unit": "Hz", "group": "frequency", "nominal": m.freq_pseudo }));
    }
    for (j, m) in a0.modes.iter().enumerate() {
        quantities.push(json!({ "key": format!("tilt{}", j + 1), "name": format!("mode {} (≈{}) axis tilt", j + 1, axis_name(m.axis)), "unit": "rad", "group": "tilt", "nominal": 0.0, "axis": m.axis }));
    }
    let nq = quantities.len();
    let nv = nominal.values.len();

    // per electrode and degree of freedom: first (and second) derivatives
    let mut params = Vec::new();
    let mut d1: Vec<Vec<Option<f64>>> = Vec::new();
    let mut d2: Vec<Vec<Option<f64>>> = Vec::new();
    let find = |e: usize, dof: usize, s: f64| data.cases.iter().position(|c| c.electrode == e && c.dof == dof && c.sign == s).and_then(|i| readings[i].as_ref());
    for (e, name) in data.electrodes.iter().enumerate() {
        for dof in 0..6 {
            let h = data.step(dof);
            let plus = find(e, dof, 1.0);
            let minus = if data.central { find(e, dof, -1.0) } else { None };
            if plus.is_none() && !(data.central && minus.is_some()) {
                continue; // not computed (stopped early) or not confined
            }
            params.push(json!({
                "electrode": name,
                "dof": dof,
                "label": DOFS[dof],
                "kind": if dof < 3 { "translate" } else { "rotate" },
                "step": h,
                "pivot": data.pivots[e],
            }));
            let mut row1 = vec![None; nq];
            let mut row2 = vec![None; nq];
            match (plus, minus, data.central) {
                (Some(p), Some(m), true) => {
                    for q in 0..nv {
                        row1[q] = finite((p.values[q] - m.values[q]) / (2.0 * h));
                        row2[q] = finite((p.values[q] + m.values[q] - 2.0 * nominal.values[q]) / (h * h));
                    }
                    for j in 0..3 {
                        let d = [0, 1, 2].map(|k| (p.axes[j][k] - m.axes[j][k]) / (2.0 * h));
                        row1[nv + j] = finite(dot(d, d).sqrt());
                    }
                }
                // one side only: forward (or backward) difference
                (p, m, _) => {
                    let (r, s) = match (p, m) {
                        (Some(r), _) => (r, 1.0),
                        (None, Some(r)) => (r, -1.0),
                        _ => unreachable!(),
                    };
                    for q in 0..nv {
                        row1[q] = finite(s * (r.values[q] - nominal.values[q]) / h);
                    }
                    for j in 0..3 {
                        let d = [0, 1, 2].map(|k| (r.axes[j][k] - nominal.axes[j][k]) / h);
                        row1[nv + j] = finite(dot(d, d).sqrt());
                    }
                }
            }
            d1.push(row1);
            d2.push(row2);
        }
    }
    // quantities without a meaning here: the null's position along a null line, and the
    // axes of (nearly) degenerate modes, which any perturbation turns by a large angle
    let mut undefined = vec![false; nq];
    if let Some(l) = a0.rf_null_line {
        notes.push(format!("The RF null is a line along ({:.3}, {:.3}, {:.3}): its shifts are those of the line where it passes the ion.", l[0], l[1], l[2]));
        for k in 0..3 {
            if l[k].abs() > 0.9 {
                undefined[3 + k] = true;
            }
        }
    }
    let f0: Vec<f64> = a0.modes.iter().map(|m| m.freq_pseudo.unwrap_or(f64::NAN)).collect();
    for i in 0..3 {
        for j in (i + 1)..3 {
            let split = (f0[i] - f0[j]).abs() / f0[i].max(f0[j]);
            if split < 0.03 {
                notes.push(format!(
                    "Modes {} and {} are nearly degenerate ({:.2} % apart): their axes turn freely under small perturbations, so their tilts are left out.",
                    i + 1,
                    j + 1,
                    100.0 * split
                ));
                undefined[nv + i] = true;
                undefined[nv + j] = true;
            }
        }
    }
    for row in d1.iter_mut().chain(d2.iter_mut()) {
        for (x, &u) in row.iter_mut().zip(&undefined) {
            if u {
                *x = None;
            }
        }
    }
    for (q, &u) in quantities.iter_mut().zip(&undefined) {
        if u {
            q["undefined"] = json!(true);
        }
    }
    Ok(json!({
        "quantities": quantities,
        "params": params,
        "d1": d1,
        "d2": data.central.then_some(d2),
        "central": data.central,
        "translate_step": data.translate_step,
        "rotate_step": data.rotate_step,
        "notes": notes,
        "seconds": t0.elapsed().as_secs_f64(),
    }))
}

/// Standard deviations of independent, normally distributed displacements: every
/// translation component (m) and every rotation component (rad) of every electrode.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Sigma {
    #[serde(default)]
    pub translate: f64,
    #[serde(default)]
    pub rotate: f64,
}

/// Tolerance budget from an [`evaluate`] result: per quantity the standard deviation of
/// its change to first order (Σ (∂q/∂p σ_p)² summed in quadrature), the mean shift from
/// the second derivatives (½ Σ ∂²q/∂p² σ_p², central differences only) and the largest
/// contributions.
pub fn budget(eval: &Value, sigma: Sigma) -> Value {
    let params = eval["params"].as_array().cloned().unwrap_or_default();
    let quantities = eval["quantities"].as_array().cloned().unwrap_or_default();
    let s: Vec<f64> = params.iter().map(|p| if p["kind"] == "translate" { sigma.translate } else { sigma.rotate }).collect();
    let at = |m: &str, p: usize, q: usize| eval[m][p][q].as_f64();
    let rows: Vec<Value> = quantities
        .iter()
        .enumerate()
        .map(|(q, quantity)| {
            let mut parts: Vec<(f64, usize)> = (0..params.len()).filter_map(|p| at("d1", p, q).map(|d| ((d * s[p]).powi(2), p))).collect();
            let var: f64 = parts.iter().map(|x| x.0).sum();
            parts.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            let second: Vec<f64> = (0..params.len()).filter_map(|p| at("d2", p, q).map(|d| d * s[p] * s[p])).collect();
            let shift = (!second.is_empty()).then(|| 0.5 * second.iter().sum::<f64>());
            let top: Vec<Value> = parts
                .iter()
                .take(3)
                .filter(|x| var > 0.0 && x.0 > 0.0)
                .map(|&(v, p)| json!({ "electrode": params[p]["electrode"], "dof": params[p]["label"], "share": v / var }))
                .collect();
            let sigma = (!parts.is_empty()).then(|| var.sqrt());
            json!({ "key": quantity["key"], "name": quantity["name"], "unit": quantity["unit"], "sigma": sigma, "mean_shift": shift, "top": top })
        })
        .collect();
    json!({ "sigma": { "translate": sigma.translate, "rotate": sigma.rotate }, "rows": rows })
}
