//! Project file (`trapfields.json`) in the project directory. The STEP file is never
//! modified; transforms, feature removals and labels are stored here and re-applied on load.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tf_geom::{FeatureRemoval, MeshParams, TransformOp};

pub const PROJECT_FILE: &str = "trapfields.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    Unassigned,
    Rf,
    Dc,
    Ground,
    Floating,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Label {
    /// Electrode this body belongs to (bodies with the same name are one electrode).
    pub electrode: String,
    #[serde(default)]
    pub role: Role,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Project {
    #[serde(default = "version")]
    pub version: u32,
    pub step_file: Option<String>,
    #[serde(default)]
    pub transforms: Vec<TransformOp>,
    /// Features (holes, threads) removed from the bodies, in order; re-applied after
    /// every import of the STEP file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_features: Vec<FeatureRemoval>,
    /// Keyed by body name.
    #[serde(default)]
    pub labels: BTreeMap<String, Label>,
    pub mesh_params: Option<MeshParams>,
    #[serde(default)]
    pub solve_params: Option<crate::solve::SolveParams>,
    #[serde(default)]
    pub trap_params: Option<crate::solve::TrapParams>,
    /// Rigid displacements of electrodes (by electrode name), applied to their panels
    /// before solving: what-if studies of machining and assembly errors.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub moves: crate::moves::Moves,
}

fn version() -> u32 {
    1
}

impl Project {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join(PROJECT_FILE)
    }

    pub fn load(dir: &Path) -> anyhow::Result<Project> {
        let p = Self::path(dir);
        if !p.exists() {
            return Ok(Project { version: 1, ..Default::default() });
        }
        Ok(serde_json::from_str(&std::fs::read_to_string(&p)?)?)
    }

    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        let tmp = dir.join(format!("{PROJECT_FILE}.tmp"));
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(tmp, Self::path(dir))?;
        Ok(())
    }
}

/// STEP files directly inside the project directory.
pub fn step_files(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let lower = name.to_lowercase();
            (e.path().is_file() && (lower.ends_with(".step") || lower.ends_with(".stp"))).then_some(name)
        })
        .collect();
    v.sort();
    v
}
