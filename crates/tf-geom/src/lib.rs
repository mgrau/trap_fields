//! Trap geometry pipeline: STEP import with electrode names, rigid re-orientation,
//! display tessellation and graded surface meshing for the BEM solver.

pub mod defeature;
pub mod mesh;
pub mod metrics;
pub mod model;
pub mod step;
pub mod transform;

pub use defeature::{FaceRef, FeatureRemoval};
pub use mesh::{mesh_bem, MeshParams, MeshStats, SurfaceMesh};
pub use model::{load_step, load_step_monitored, no_monitor, tessellate, tessellate_monitored, Body, Face, GeomError, Model, Monitor, Report, Result, Tessellation};
pub use transform::{Rigid, TransformOp};
