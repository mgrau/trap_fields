//! Field export: potentials and fields on a regular grid, as CSV, for analyses outside the
//! app. Values come from the surface charges directly (valid everywhere, inside the
//! electrodes too, where φ is the electrode's potential and E vanishes).

use crate::solve::{SolveResult, TrapParams};
use rayon::prelude::*;
use serde::Deserialize;
use std::fmt::Write;
use tf_bem::trap::{inside_surface, E_CHARGE};
use tf_bem::V3;

/// Most grid points per export (a few seconds natively on a trap mesh).
pub const MAX_EXPORT_POINTS: usize = 125_000;

/// Regular grid: `n[a]` nodes along axis a from center − half to center + half (one node: the center).
#[derive(Debug, Clone, Deserialize)]
pub struct Grid {
    pub center: V3,
    pub half: [f64; 3],
    pub n: [usize; 3],
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FieldSet {
    /// One basis solution per electrode (1 V on it, 0 V on the others, floating ones free):
    /// any drive is a weighted sum of these.
    Basis,
    /// The RF amplitude and static potentials of one drive, and the trapping energy U.
    Drive { params: TrapParams },
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExportArgs {
    pub grid: Grid,
    pub fields: FieldSet,
}

impl Grid {
    fn points(&self) -> anyhow::Result<Vec<V3>> {
        anyhow::ensure!(self.n.iter().all(|&k| k >= 1), "every axis needs at least one grid node");
        anyhow::ensure!(self.half.iter().all(|&h| h >= 0.0 && h.is_finite()), "half-widths must be non-negative");
        let total = self.n.iter().try_fold(1usize, |a, &k| a.checked_mul(k)).unwrap_or(usize::MAX);
        anyhow::ensure!(total <= MAX_EXPORT_POINTS, "{total} grid points; at most {MAX_EXPORT_POINTS} per export");
        let coord = |a: usize, i: usize| {
            let n = self.n[a];
            if n == 1 {
                self.center[a]
            } else {
                self.center[a] + self.half[a] * (2.0 * i as f64 / (n - 1) as f64 - 1.0)
            }
        };
        // x fastest, then y, then z
        let mut pts = Vec::with_capacity(total);
        for k in 0..self.n[2] {
            for j in 0..self.n[1] {
                for i in 0..self.n[0] {
                    pts.push([coord(0, i), coord(1, j), coord(2, k)]);
                }
            }
        }
        Ok(pts)
    }
}

/// Column-name-safe version of an electrode name.
fn column(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    s.trim_matches('_').to_string()
}

fn num(out: &mut String, x: f64) {
    if x.is_finite() {
        let _ = write!(out, ",{x:.9e}");
    } else {
        out.push_str(",nan");
    }
}

/// The grid as CSV: x_m, y_m, z_m, inside (1 inside an electrode), then per field
/// set φ (V) and E = −∇φ (V/m) — and U (eV) for a drive.
pub fn export_csv(r: &SolveResult, a: &ExportArgs) -> anyhow::Result<Vec<u8>> {
    let pts = a.grid.points()?;
    let panels = &r.problem.panels;
    let inside: Vec<bool> = pts.par_iter().map(|&x| inside_surface(panels, x)).collect();
    let mut head = String::from("x_m,y_m,z_m,inside");
    let (densities, drive): (Vec<Vec<f64>>, Option<&TrapParams>) = match &a.fields {
        FieldSet::Basis => {
            let mut names = Vec::new();
            for &k in &r.solution.basis {
                let mut c = column(&r.electrodes[k].name);
                if c.is_empty() || names.contains(&c) {
                    c = format!("e{k}");
                }
                let _ = write!(head, ",phi_{c}_V,Ex_{c}_V_per_m,Ey_{c}_V_per_m,Ez_{c}_V_per_m");
                names.push(c);
            }
            (r.solution.u.clone(), None)
        }
        FieldSet::Drive { params } => {
            r.drive(params)?; // validates the parameters and electrode names
            let (u_rf, _) = r.combine(&params.rf)?;
            let (u_dc, _) = r.combine(&params.dc)?;
            for c in ["rf", "dc"] {
                let _ = write!(head, ",phi_{c}_V,Ex_{c}_V_per_m,Ey_{c}_V_per_m,Ez_{c}_V_per_m");
            }
            head.push_str(",U_eV");
            (vec![u_rf, u_dc], Some(params))
        }
    };
    let us: Vec<&[f64]> = densities.iter().map(|u| u.as_slice()).collect();
    let vals = tf_bem::evaluate_many(panels, &us, &pts);
    let pp = drive.map(|p| {
        let w = 2.0 * std::f64::consts::PI * p.rf_freq;
        (p.ion.charge().powi(2) / (4.0 * p.ion.mass() * w * w), p.ion.charge())
    });
    let mut out = String::with_capacity(pts.len() * (40 + 64 * us.len()));
    out.push_str(&head);
    out.push('\n');
    for ((x, v), &ins) in pts.iter().zip(&vals).zip(&inside) {
        let _ = write!(out, "{:.9e},{:.9e},{:.9e},{}", x[0], x[1], x[2], ins as u8);
        for (phi, g) in v {
            num(&mut out, *phi);
            for gk in g {
                num(&mut out, -gk);
            }
        }
        if let Some((c, q)) = pp {
            let g = v[0].1;
            num(&mut out, (c * (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]) + q * v[1].0) / E_CHARGE);
        }
        out.push('\n');
    }
    Ok(out.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_order_and_limits() {
        let g = Grid { center: [1.0, 2.0, 3.0], half: [1.0, 0.5, 9.0], n: [3, 2, 1] };
        let p = g.points().unwrap();
        assert_eq!(p.len(), 6);
        assert_eq!(p[0], [0.0, 1.5, 3.0]);
        assert_eq!(p[1], [1.0, 1.5, 3.0]);
        assert_eq!(p[3], [0.0, 2.5, 3.0]);
        assert!(Grid { center: [0.0; 3], half: [1.0; 3], n: [100, 100, 100] }.points().is_err());
        assert!(Grid { center: [0.0; 3], half: [1.0; 3], n: [0, 1, 1] }.points().is_err());
    }

    #[test]
    fn column_names() {
        assert_eq!(column("Endcap 1"), "Endcap_1");
        assert_eq!(column("RF,1"), "RF_1");
        assert_eq!(column("  "), "");
    }
}
