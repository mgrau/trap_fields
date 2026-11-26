//! Rigid displacements of single electrodes (machining and assembly errors): a translation
//! and a small rotation per electrode, applied to its panels after meshing. The mesh is
//! generated on the nominal geometry and moves with the electrode, so a displaced solve
//! uses the same panels as the nominal one and differences between the two are not
//! buried in discretization noise.

use crate::solve::Electrode;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tf_bem::{Panel, Problem, V3};
use tf_geom::{Model, Rigid, TransformOp};

/// Displacement of one electrode, in the trap frame.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Move {
    /// Translation (m).
    #[serde(default)]
    pub translate: V3,
    /// Rotation vector (rad): a rotation by |rotate| about rotate/|rotate|, through `pivot`.
    #[serde(default)]
    pub rotate: V3,
    /// Point the rotation turns about (m); default: the electrode's center (see [`centers`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pivot: Option<V3>,
}

/// Displacements by electrode name.
pub type Moves = BTreeMap<String, Move>;

fn norm(v: V3) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

impl Move {
    pub fn is_zero(&self) -> bool {
        self.translate == [0.0; 3] && self.rotate == [0.0; 3]
    }

    pub fn validate(&self, name: &str) -> anyhow::Result<()> {
        let finite = self.translate.iter().chain(&self.rotate).chain(self.pivot.iter().flatten()).all(|x| x.is_finite());
        anyhow::ensure!(finite, "the displacement of '{name}' is not a finite number");
        anyhow::ensure!(norm(self.translate) < 1.0, "the displacement of '{name}' is 1 m or more");
        anyhow::ensure!(norm(self.rotate) <= std::f64::consts::PI, "the rotation of '{name}' exceeds 180°");
        Ok(())
    }

    /// x ↦ R (x − p) + p + t, with p the pivot (`center` unless the move names one).
    pub fn rigid(&self, center: V3) -> Rigid {
        let p = self.pivot.unwrap_or(center);
        let th = norm(self.rotate);
        let rot = if th > 0.0 { Rigid::from_op(&TransformOp::Rotate { axis: self.rotate, angle_deg: th.to_degrees(), center: p }) } else { Rigid::IDENTITY };
        rot.then(&Rigid::from_op(&TransformOp::Translate { delta: self.translate }))
    }
}

/// The moves that apply: non-zero ones of electrodes that exist (an electrode renamed on
/// the Electrodes tab leaves its old entry behind, which then does nothing).
pub fn effective(moves: &Moves, electrodes: &[Electrode]) -> Moves {
    moves.iter().filter(|(n, m)| !m.is_zero() && electrodes.iter().any(|e| &e.name == *n)).map(|(n, m)| (n.clone(), *m)).collect()
}

/// Center of each electrode (m, trap frame, before any move): the volume-weighted center
/// of mass of its bodies. Rotations turn about it unless a move names its own pivot.
pub fn centers(model: &Model, frame: &Rigid, electrodes: &[Electrode]) -> Vec<V3> {
    electrodes
        .iter()
        .map(|e| {
            let (mut c, mut w) = ([0.0; 3], 0.0);
            for b in model.bodies.iter().filter(|b| e.bodies.contains(&b.name)) {
                let v = b.volume.abs().max(1e-30);
                let p = frame.apply(b.center_of_mass);
                (0..3).for_each(|k| c[k] += v * p[k]);
                w += v;
            }
            if w > 0.0 {
                c.map(|x| x / w)
            } else {
                [0.0; 3]
            }
        })
        .collect()
}

/// Rigid motion of each electrode (None: it stays), in the order of `electrodes`.
pub fn rigids(moves: &Moves, electrodes: &[Electrode], centers: &[V3]) -> Vec<Option<Rigid>> {
    electrodes.iter().zip(centers).map(|(e, &c)| moves.get(&e.name).filter(|m| !m.is_zero()).map(|m| m.rigid(c))).collect()
}

/// Move the panels of every electrode that has a rigid motion (indexed by conductor).
pub fn apply(problem: &mut Problem, rigids: &[Option<Rigid>]) -> anyhow::Result<()> {
    let conductor = &problem.panel_conductor;
    problem.panels.par_iter_mut().zip(conductor.par_iter()).try_for_each(|(p, &k)| {
        if let Some(r) = rigids.get(k).copied().flatten() {
            let [a, b, c] = p.v.map(|v| r.apply(v));
            *p = Panel::new(a, b, c).ok_or_else(|| anyhow::anyhow!("a moved panel became degenerate"))?;
        }
        Ok(())
    })
}

/// Closest approach between electrode `k` and any other electrode: (distance between
/// panel centroids (m), the other electrode, the larger panel radius of that pair).
/// Panels farther than `reach` from k's bounding box are not looked at.
pub fn closest_approach(problem: &Problem, k: usize, reach: f64) -> Option<(f64, usize, f64)> {
    let mine: Vec<&Panel> = problem.panels.iter().zip(&problem.panel_conductor).filter(|(_, &c)| c == k).map(|(p, _)| p).collect();
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in &mine {
        for i in 0..3 {
            lo[i] = lo[i].min(p.centroid[i]);
            hi[i] = hi[i].max(p.centroid[i]);
        }
    }
    let others: Vec<(&Panel, usize)> = problem
        .panels
        .iter()
        .zip(&problem.panel_conductor)
        .filter(|(p, &c)| c != k && (0..3).all(|i| p.centroid[i] >= lo[i] - reach && p.centroid[i] <= hi[i] + reach))
        .map(|(p, &c)| (p, c))
        .collect();
    others
        .par_iter()
        .map(|&(q, c)| {
            let (d, r) = mine.iter().map(|p| (norm([p.centroid[0] - q.centroid[0], p.centroid[1] - q.centroid[1], p.centroid[2] - q.centroid[2]]), p.radius)).fold((f64::INFINITY, 0.0), |a, b| if b.0 < a.0 { b } else { a });
            (d, c, r.max(q.radius))
        })
        .reduce_with(|a, b| if b.0 < a.0 { b } else { a })
}

/// Warnings for moved electrodes that come so close to another electrode that they may
/// touch or overlap (closer than the panels there are large).
pub fn contact_warnings(problem: &Problem, rigids: &[Option<Rigid>], electrodes: &[Electrode]) -> Vec<String> {
    let mut out = Vec::new();
    for (k, r) in rigids.iter().enumerate() {
        let Some(r) = r else { continue };
        // how far this electrode's panels moved, at most: the reach of the search
        let moved = problem
            .panels
            .iter()
            .zip(&problem.panel_conductor)
            .filter(|(_, &c)| c == k)
            .map(|(p, _)| {
                let back = r.inverse().apply(p.centroid);
                norm([p.centroid[0] - back[0], p.centroid[1] - back[1], p.centroid[2] - back[2]]) + 2.0 * p.radius
            })
            .fold(0.0, f64::max);
        if let Some((d, j, size)) = closest_approach(problem, k, moved) {
            if d < size {
                out.push(format!(
                    "After its move, '{}' comes within {:.1} µm of '{}' (panels there are {:.1} µm): the electrodes may touch or overlap, and the solution is not reliable.",
                    electrodes[k].name,
                    d * 1e6,
                    electrodes[j].name,
                    size * 1e6
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_turns_about_the_pivot_then_translates() {
        let m = Move { translate: [1e-6, 0.0, 0.0], rotate: [0.0, 0.0, std::f64::consts::FRAC_PI_2], pivot: None };
        let r = m.rigid([1.0, 0.0, 0.0]);
        // the pivot only translates
        let p = r.apply([1.0, 0.0, 0.0]);
        assert!((p[0] - (1.0 + 1e-6)).abs() < 1e-12 && p[1].abs() < 1e-12);
        // a point one unit beyond it turns by 90° about z
        let q = r.apply([2.0, 0.0, 0.0]);
        assert!((q[0] - (1.0 + 1e-6)).abs() < 1e-12 && (q[1] - 1.0).abs() < 1e-12, "{q:?}");
        assert!(Move::default().is_zero());
        assert!(Move { rotate: [4.0, 0.0, 0.0], ..Default::default() }.validate("e").is_err());
    }
}
