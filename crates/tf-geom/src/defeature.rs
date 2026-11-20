//! Removing features (screw holes, threads, chamfers) from a body with OpenCASCADE's
//! defeaturing: the faces of the feature are deleted and the neighboring faces extended
//! to close the gap. They barely change the field at the ion but cost many panels, and
//! modeled threads may not mesh at all.
//!
//! The STEP file is never modified: removals are stored in the project and re-applied
//! after every import. gmsh's face tags are not a stable identity (they are renumbered
//! when a volume is rebuilt, and could differ between gmsh/OpenCASCADE builds), so a
//! removal names its body and identifies each face by its centroid and area, taken in
//! the STEP frame on the model as it was when the removal was made (i.e. after the
//! removals before it in the list).

use crate::model::{GeomError, Model, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use tf_gmsh::Gmsh;

/// A face by its geometry (STEP frame).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FaceRef {
    /// Centroid (m).
    pub center: [f64; 3],
    /// m^2
    pub area: f64,
}

/// One defeaturing operation on one body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeatureRemoval {
    /// Body name (as in the labels).
    pub body: String,
    pub faces: Vec<FaceRef>,
}

fn face_ref(g: &Gmsh, tag: i32) -> Result<FaceRef> {
    Ok(FaceRef { center: g.occ_center_of_mass(2, tag)?, area: g.occ_mass(2, tag)? })
}

fn diag(b: &[f64; 6]) -> f64 {
    ((0..3).map(|k| (b[k + 3] - b[k]).powi(2)).sum::<f64>()).sqrt()
}

/// The body that owns all of `faces` (each body's own face list, so a face shared by
/// touching bodies counts for both).
fn owner(model: &Model, faces: &[i32]) -> Result<usize> {
    let first = *faces.first().ok_or_else(|| GeomError::Invalid("no faces given".into()))?;
    model
        .bodies
        .iter()
        .position(|b| faces.iter().all(|f| b.faces.contains(f)))
        .ok_or_else(|| {
            if model.bodies.iter().any(|b| b.faces.contains(&first)) {
                GeomError::Invalid("the faces of one removal must all belong to one body".into())
            } else {
                GeomError::Invalid(format!("face {first} is not part of the model"))
            }
        })
}

/// Describe faces of the current model as a removal (to store in the project).
pub fn describe(g: &Gmsh, model: &Model, faces: &[i32]) -> Result<FeatureRemoval> {
    let b = owner(model, faces)?;
    let set: BTreeSet<i32> = faces.iter().copied().collect();
    let faces = set.into_iter().map(|f| face_ref(g, f)).collect::<Result<_>>()?;
    Ok(FeatureRemoval { body: model.bodies[b].name.clone(), faces })
}

/// Find the faces a stored removal refers to: per reference, the body's face with the
/// nearest centroid, within a tolerance relative to the body size and with a matching area.
pub fn resolve(g: &Gmsh, model: &Model, r: &FeatureRemoval) -> Result<(usize, Vec<i32>)> {
    let bi = model.bodies.iter().position(|b| b.name == r.body).ok_or_else(|| GeomError::Invalid(format!("no body named '{}'", r.body)))?;
    let body = &model.bodies[bi];
    let tol = 1e-5 * diag(&g.bounding_box(3, body.tag)?).max(1e-6);
    let refs: Vec<(i32, FaceRef)> = body.faces.iter().map(|&f| Ok((f, face_ref(g, f)?))).collect::<Result<_>>()?;
    let mut out = Vec::with_capacity(r.faces.len());
    for want in &r.faces {
        let best = refs
            .iter()
            .filter(|(_, have)| (have.area - want.area).abs() <= 1e-4 * want.area.max(have.area) + tol * tol)
            .map(|(f, have)| (*f, (0..3).map(|k| (have.center[k] - want.center[k]).powi(2)).sum::<f64>().sqrt()))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        match best {
            Some((f, d)) if d <= tol => out.push(f),
            _ => {
                let c = want.center.map(|x| format!("{:.3}", x * 1e3)).join(", ");
                return Err(GeomError::Invalid(format!("a face of '{}' (area {:.3} mm², center {c} mm) is no longer in the model", r.body, want.area * 1e6)));
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok((bi, out))
}

/// Remove `faces` from body `bi`. If OpenCASCADE refuses, gmsh's model is unchanged; an
/// error about an unusable result comes after gmsh took that result, so callers that need
/// the old model back reload it.
pub fn remove_faces(g: &mut Gmsh, model: &mut Model, bi: usize, faces: &[i32]) -> Result<()> {
    let tag = model.bodies[bi].tag;
    let (n_before, v_before) = (model.bodies[bi].n_faces, model.bodies[bi].volume);
    let out = g.occ_defeature(&[tag], faces, true).map_err(|e| GeomError::Invalid(format!("OpenCASCADE could not remove these faces from '{}': {}", model.bodies[bi].name, e.0)))?;
    g.occ_synchronize()?;
    let vols: Vec<i32> = out.iter().filter(|d| d.0 == 3).map(|d| d.1).collect();
    let [new] = vols[..] else {
        return Err(GeomError::Invalid(format!("defeaturing '{}' produced {} volumes instead of one", model.bodies[bi].name, vols.len())));
    };
    model.bodies[bi].tag = new;
    model.refresh_topology(g)?;
    let b = &model.bodies[bi];
    if !(b.volume > 0.0) || b.n_faces == 0 {
        return Err(GeomError::Invalid(format!("defeaturing '{}' produced an empty solid", b.name)));
    }
    // OpenCASCADE sometimes "succeeds" by returning the solid unchanged
    if b.n_faces == n_before && (b.volume - v_before).abs() <= 1e-9 * v_before {
        return Err(GeomError::Invalid(format!("OpenCASCADE could not remove these faces from '{}' (the solid came back unchanged)", b.name)));
    }
    Ok(())
}

/// Re-apply a stored removal.
pub fn apply(g: &mut Gmsh, model: &mut Model, r: &FeatureRemoval) -> Result<()> {
    let (bi, faces) = resolve(g, model, r)?;
    remove_faces(g, model, bi, &faces)
}

/// The faces that make up the feature `seed` belongs to (a hole with its chamfers,
/// countersink and bottom; a modeled thread), from the B-rep topology. Starting at the
/// seed, grow across shared edges into
/// - curved faces of a similar size (bounding-box diagonal at most 4× the seed's and below
///   half the body's): the walls, chamfers and thread flanks of the feature, but not the
///   body's own large curved surfaces;
/// - planar faces every edge of which is shared with faces already in the feature (a
///   blind hole's bottom, a counterbore's annular step), but not the face the hole is
///   drilled into, whose outer edges belong to the rest of the body.
pub fn grow_feature(g: &Gmsh, model: &Model, seed: i32) -> Result<Vec<i32>> {
    let bi = owner(model, &[seed])?;
    let body = &model.bodies[bi];
    let in_body: BTreeSet<i32> = body.faces.iter().copied().collect();
    let body_diag = diag(&g.bounding_box(3, body.tag)?);
    let seed_diag = diag(&g.bounding_box(2, seed)?);

    let mut curves_of: HashMap<i32, Vec<i32>> = HashMap::new();
    let mut faces_of: HashMap<i32, Vec<i32>> = HashMap::new();
    let curves = |f: i32, curves_of: &mut HashMap<i32, Vec<i32>>| -> Result<Vec<i32>> {
        if let Some(c) = curves_of.get(&f) {
            return Ok(c.clone());
        }
        let c: Vec<i32> = g.adjacencies(2, f)?.1.into_iter().map(i32::abs).collect();
        curves_of.insert(f, c.clone());
        Ok(c)
    };
    let faces_on = |c: i32, faces_of: &mut HashMap<i32, Vec<i32>>| -> Result<Vec<i32>> {
        if let Some(f) = faces_of.get(&c) {
            return Ok(f.clone());
        }
        let f: Vec<i32> = g.adjacencies(1, c)?.0.into_iter().map(i32::abs).filter(|f| in_body.contains(f)).collect();
        faces_of.insert(c, f.clone());
        Ok(f)
    };

    let mut sel: BTreeSet<i32> = BTreeSet::from([seed]);
    let mut queue = vec![seed];
    let mut planar: BTreeSet<i32> = BTreeSet::new();
    loop {
        while let Some(f) = queue.pop() {
            for c in curves(f, &mut curves_of)? {
                for n in faces_on(c, &mut faces_of)? {
                    if sel.contains(&n) || planar.contains(&n) {
                        continue;
                    }
                    if g.entity_type(2, n)? == "Plane" {
                        planar.insert(n);
                        continue;
                    }
                    let d = diag(&g.bounding_box(2, n)?);
                    if d <= 4.0 * seed_diag && d < 0.5 * body_diag {
                        sel.insert(n);
                        queue.push(n);
                    }
                }
            }
        }
        // planar faces closed off by the feature; they may in turn border more of it
        let mut added = false;
        for &p in &planar.clone() {
            if diag(&g.bounding_box(2, p)?) >= 0.5 * body_diag {
                continue;
            }
            let mut closed = true;
            for c in curves(p, &mut curves_of)? {
                if !faces_on(c, &mut faces_of)?.iter().any(|n| *n != p && sel.contains(n)) {
                    closed = false;
                    break;
                }
            }
            if closed {
                planar.remove(&p);
                sel.insert(p);
                queue.push(p);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    Ok(sel.into_iter().collect())
}
